<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  interface Device {
    name: string;
    ip: string;
    cmd_port: number;
    audio_port: number;
  }

  interface ConnectionStatus {
    connected: boolean;
    ip: string | null;
    port: number | null;
  }

  interface DeviceStatus {
    protocol_ver: number;
    wifi_mode: string;
    brightness: number;
    color_r: number;
    color_g: number;
    color_b: number;
    speed: number;
    direction: number;
    grid_width: number;
    grid_height: number;
    num_pixels: number;
    effect: string;
    palette: string;
  }

  function applyStatus(status: DeviceStatus) {
    currentEffect = status.effect;
    brightness = status.brightness;
    speed = status.speed;
    direction = Math.round(status.direction);
    colorR = status.color_r;
    colorG = status.color_g;
    colorB = status.color_b;
    currentPalette = status.palette;
  }

  onMount(() => {
    listen<DeviceStatus>("device-status", (event) => {
      applyStatus(event.payload);
    });
    listen("device-disconnected", () => {
      connected = false;
      connectedIp = "";
      statusMsg = "Device disconnected";
    });
  });

  let devices = $state<Device[]>([]);
  let scanning = $state(false);
  let connected = $state(false);
  let connectedIp = $state("");
  let statusMsg = $state("");

  // Control state
  let currentEffect = $state("rainbow");
  let brightness = $state(10);
  let speed = $state(0.0);
  let direction = $state(0);
  let colorR = $state(0);
  let colorG = $state(0);
  let colorB = $state(255);
  let currentPalette = $state("rainbow");

  // Audio state
  interface AudioDevice {
    name: string;
    index: number;
  }

  let audioDevices = $state<AudioDevice[]>([]);
  let selectedDeviceIndex = $state(0);
  let audioStreaming = $state(false);
  let audioStatusMsg = $state("");

  // WiFi setup state
  let showManualConnect = $state(false);
  let manualIp = $state("192.168.4.1");
  let manualPort = $state(5001);
  let wifiSsid = $state("");
  let wifiPassword = $state("");
  let wifiStatusMsg = $state("");

  const effects = [
    "rainbow", "bass", "twinkle", "solid", "splash",
    "fire", "breathe", "wipe", "spectrum", "image"
  ];

  const palettes = ["rainbow", "fire"];

  async function scan() {
    scanning = true;
    statusMsg = "Scanning...";
    try {
      devices = await invoke("discover_devices");
      statusMsg = `Found ${devices.length} device(s)`;
    } catch (e) {
      statusMsg = `Scan failed: ${e}`;
    }
    scanning = false;
  }

  async function connectDevice(device: Device) {
    try {
      const status: DeviceStatus = await invoke("connect", { ip: device.ip, port: device.cmd_port });
      connected = true;
      connectedIp = device.ip;
      applyStatus(status);
      statusMsg = `Connected to ${device.name} (${device.ip})`;
    } catch (e) {
      statusMsg = `Connect failed: ${e}`;
    }
  }

  async function disconnectDevice() {
    try {
      await invoke("disconnect");
    } catch (_) {}
    connected = false;
    connectedIp = "";
    statusMsg = "Disconnected";
  }

  async function sendCmd(cmd: string): Promise<string> {
    try {
      const resp: string = await invoke("send_command", { cmd });
      return resp.trim();
    } catch (e) {
      statusMsg = `Error: ${e}`;
      if (String(e).includes("Not connected") || String(e).includes("Connection closed")) {
        connected = false;
        connectedIp = "";
      }
      return "";
    }
  }

  async function setEffect(name: string) {
    currentEffect = name;
    await sendCmd(`effect ${name}`);
  }

  async function setBrightness() {
    await sendCmd(`brightness ${brightness}`);
  }

  async function setSpeed() {
    await sendCmd(`speed ${speed.toFixed(2)}`);
  }

  async function setDirection() {
    await sendCmd(`direction ${direction}`);
  }

  async function setColor() {
    await sendCmd(`color ${colorR} ${colorG} ${colorB}`);
  }

  async function setPalette(name: string) {
    currentPalette = name;
    await sendCmd(`palette ${name}`);
  }

  // Manual / AP mode connection
  async function connectManual() {
    try {
      const status: DeviceStatus = await invoke("connect", { ip: manualIp, port: manualPort });
      connected = true;
      connectedIp = manualIp;
      applyStatus(status);
      statusMsg = `Connected to ${manualIp}`;
    } catch (e) {
      statusMsg = `Connect failed: ${e}`;
    }
  }

  async function configureWifi() {
    if (!wifiSsid || !wifiPassword) {
      wifiStatusMsg = "SSID and password are required";
      return;
    }
    wifiStatusMsg = "Saving credentials...";
    const resp = await sendCmd(`wifi ${wifiSsid} ${wifiPassword}`);
    wifiStatusMsg = resp || "Credentials sent — device is rebooting. Reconnect after it joins your network.";
    // The device will reboot, so we'll lose connection
    connected = false;
    connectedIp = "";
  }

  // Audio functions
  async function loadAudioDevices() {
    try {
      audioDevices = await invoke("list_audio_devices");
      if (audioDevices.length > 0) {
        selectedDeviceIndex = audioDevices[0].index;
      }
    } catch (e) {
      audioStatusMsg = `Failed to list devices: ${e}`;
    }
  }

  async function startAudio() {
    try {
      const info: string = await invoke("start_audio_stream", {
        ip: connectedIp,
        deviceIndex: selectedDeviceIndex,
      });
      audioStreaming = true;
      audioStatusMsg = info;
    } catch (e) {
      audioStatusMsg = `Start failed: ${e}`;
    }
  }

  async function stopAudio() {
    try {
      await invoke("stop_audio_stream");
      audioStreaming = false;
      audioStatusMsg = "Audio stopped";
    } catch (e) {
      audioStatusMsg = `Stop failed: ${e}`;
    }
  }
</script>

<main>
  <h1>PulseBox</h1>

  <!-- Connection Section -->
  <section class="panel">
    <h2>Connection</h2>
    {#if connected}
      <div class="status connected">Connected to {connectedIp}</div>
      <button onclick={disconnectDevice}>Disconnect</button>
    {:else}
      <button onclick={scan} disabled={scanning}>
        {scanning ? "Scanning..." : "Scan for Devices"}
      </button>
      {#if devices.length > 0}
        <div class="device-list">
          {#each devices as device}
            <button class="device-btn" onclick={() => connectDevice(device)}>
              {device.name} ({device.ip}:{device.cmd_port})
            </button>
          {/each}
        </div>
      {/if}
      <div class="manual-section">
        <button class="link-btn" onclick={() => showManualConnect = !showManualConnect}>
          {showManualConnect ? "Hide" : "Manual / AP Mode Connect"}
        </button>
        {#if showManualConnect}
          <div class="manual-form">
            <p class="hint">For first-time setup: connect your PC to the "PulseBox-Setup" WiFi network (password: pulsebox123), then connect to 192.168.4.1</p>
            <div class="input-row">
              <input type="text" bind:value={manualIp} placeholder="IP address" />
              <input type="number" bind:value={manualPort} style="width: 70px" />
              <button onclick={connectManual}>Connect</button>
            </div>
          </div>
        {/if}
      </div>
    {/if}
    {#if statusMsg}
      <div class="status-msg">{statusMsg}</div>
    {/if}
  </section>

  {#if connected}
    <!-- Audio Streaming -->
    <section class="panel">
      <h2>Audio Streaming</h2>
      {#if audioStreaming}
        <div class="status streaming">Streaming audio</div>
        <button onclick={stopAudio}>Stop Streaming</button>
      {:else}
        {#if audioDevices.length === 0}
          <button onclick={loadAudioDevices}>Load Audio Devices</button>
        {:else}
          <div class="audio-controls">
            <select bind:value={selectedDeviceIndex}>
              {#each audioDevices as device}
                <option value={device.index}>{device.name}</option>
              {/each}
            </select>
            <button onclick={startAudio}>Start Streaming</button>
          </div>
        {/if}
      {/if}
      {#if audioStatusMsg}
        <div class="status-msg">{audioStatusMsg}</div>
      {/if}
    </section>

    <!-- Effect Picker -->
    <section class="panel">
      <h2>Effect</h2>
      <div class="effect-grid">
        {#each effects as effect}
          <button
            class="effect-btn"
            class:active={currentEffect === effect}
            onclick={() => setEffect(effect)}
          >
            {effect}
          </button>
        {/each}
      </div>
    </section>

    <!-- Color -->
    <section class="panel">
      <h2>Color</h2>
      <div class="color-controls">
        <div class="color-preview" style="background: rgb({colorR},{colorG},{colorB})"></div>
        <div class="sliders">
          <label>R <input type="range" min="0" max="255" bind:value={colorR} onchange={setColor} /> {colorR}</label>
          <label>G <input type="range" min="0" max="255" bind:value={colorG} onchange={setColor} /> {colorG}</label>
          <label>B <input type="range" min="0" max="255" bind:value={colorB} onchange={setColor} /> {colorB}</label>
        </div>
      </div>
    </section>

    <!-- Palette -->
    <section class="panel">
      <h2>Palette</h2>
      <div class="palette-btns">
        {#each palettes as palette}
          <button
            class="palette-btn"
            class:active={currentPalette === palette}
            onclick={() => setPalette(palette)}
          >
            {palette}
          </button>
        {/each}
      </div>
    </section>

    <!-- WiFi Configuration -->
    <section class="panel">
      <h2>WiFi Configuration</h2>
      <div class="wifi-form">
        <input type="text" bind:value={wifiSsid} placeholder="WiFi SSID" />
        <input type="password" bind:value={wifiPassword} placeholder="WiFi Password" />
        <button onclick={configureWifi}>Save & Reboot</button>
      </div>
      <p class="hint">Saves credentials to the device. It will reboot and connect to this network.</p>
      {#if wifiStatusMsg}
        <div class="status-msg">{wifiStatusMsg}</div>
      {/if}
    </section>

    <!-- Controls -->
    <section class="panel">
      <h2>Controls</h2>
      <div class="sliders">
        <label>
          Brightness
          <input type="range" min="0" max="100" bind:value={brightness} onchange={setBrightness} />
          {brightness}%
        </label>
        <label>
          Speed
          <input type="range" min="0" max="1" step="0.01" bind:value={speed} onchange={setSpeed} />
          {speed.toFixed(2)}
        </label>
        <label>
          Direction
          <input type="range" min="0" max="360" bind:value={direction} onchange={setDirection} />
          {direction}&deg;
        </label>
      </div>
    </section>
  {/if}
</main>

<style>
  :root {
    font-family: Inter, system-ui, sans-serif;
    font-size: 14px;
    color: #e0e0e0;
    background-color: #1a1a2e;
  }

  main {
    max-width: 480px;
    margin: 0 auto;
    padding: 16px;
  }

  h1 {
    text-align: center;
    color: #7c3aed;
    margin-bottom: 16px;
  }

  h2 {
    margin: 0 0 8px 0;
    font-size: 0.9em;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #888;
  }

  .panel {
    background: #16213e;
    border-radius: 8px;
    padding: 12px;
    margin-bottom: 12px;
  }

  button {
    background: #0f3460;
    color: #e0e0e0;
    border: 1px solid #1a1a4e;
    border-radius: 6px;
    padding: 8px 16px;
    cursor: pointer;
    font-size: 0.9em;
  }

  button:hover {
    background: #1a4a7a;
  }

  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .status.connected {
    color: #4ade80;
    margin-bottom: 8px;
  }

  .status-msg {
    margin-top: 8px;
    font-size: 0.85em;
    color: #999;
  }

  .device-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 8px;
  }

  .device-btn {
    text-align: left;
    background: #1a2744;
  }

  .effect-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 6px;
  }

  .effect-btn {
    text-transform: capitalize;
    padding: 10px;
  }

  .effect-btn.active {
    background: #7c3aed;
    border-color: #9f67ff;
  }

  .color-controls {
    display: flex;
    gap: 12px;
    align-items: flex-start;
  }

  .color-preview {
    width: 48px;
    height: 48px;
    border-radius: 8px;
    border: 2px solid #333;
    flex-shrink: 0;
  }

  .sliders {
    display: flex;
    flex-direction: column;
    gap: 6px;
    flex: 1;
  }

  label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 0.85em;
  }

  input[type="range"] {
    flex: 1;
    accent-color: #7c3aed;
  }

  .palette-btns {
    display: flex;
    gap: 6px;
  }

  .palette-btn {
    text-transform: capitalize;
    flex: 1;
  }

  .palette-btn.active {
    background: #7c3aed;
    border-color: #9f67ff;
  }

  .status.streaming {
    color: #f59e0b;
    margin-bottom: 8px;
  }

  .audio-controls {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  select {
    flex: 1;
    background: #0f3460;
    color: #e0e0e0;
    border: 1px solid #1a1a4e;
    border-radius: 6px;
    padding: 8px;
    font-size: 0.85em;
  }

  .manual-section {
    margin-top: 8px;
  }

  .link-btn {
    background: none;
    border: none;
    color: #646cff;
    padding: 4px 0;
    font-size: 0.85em;
    text-decoration: underline;
  }

  .link-btn:hover {
    background: none;
    color: #8b8fff;
  }

  .manual-form {
    margin-top: 8px;
  }

  .input-row {
    display: flex;
    gap: 6px;
    align-items: center;
  }

  .hint {
    font-size: 0.8em;
    color: #666;
    margin: 6px 0 0 0;
  }

  input[type="text"],
  input[type="password"],
  input[type="number"] {
    background: #0f3460;
    color: #e0e0e0;
    border: 1px solid #1a1a4e;
    border-radius: 6px;
    padding: 8px;
    font-size: 0.85em;
    flex: 1;
  }

  .wifi-form {
    display: flex;
    gap: 6px;
    align-items: center;
  }
</style>
