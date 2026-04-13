<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
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

  interface PanelPosition {
    grid_x: number;
    grid_y: number;
  }

  interface DeviceStatus {
    protocol_ver: number;
    wifi_mode: string;
    brightness: number;
    color_r: number;
    color_g: number;
    color_b: number;
    color2_r: number;
    color2_g: number;
    color2_b: number;
    color3_r: number;
    color3_g: number;
    color3_b: number;
    speed: number;
    direction: number;
    grid_width: number;
    grid_height: number;
    num_pixels: number;
    sensitivity: number;
    effect: string;
    palette: string;
    panels: PanelPosition[];
  }

  function applyStatus(status: DeviceStatus) {
    currentEffect = status.effect;
    currentPalette = status.palette;
    brightness = status.brightness;
    sensitivity = status.sensitivity;
    speed = status.speed;
    direction = Math.round(status.direction);
    colorSetColors = [
      [status.color_r, status.color_g, status.color_b],
      [status.color2_r, status.color2_g, status.color2_b],
      [status.color3_r, status.color3_g, status.color3_b],
    ];
    const sel = colorSetColors[selectedColorIndex];
    colorR = sel[0]; colorG = sel[1]; colorB = sel[2];
    gridWidth = status.grid_width;
    gridHeight = status.grid_height;
    panels = status.panels && status.panels.length > 0
      ? status.panels
      : [{ grid_x: 0, grid_y: 0 }];
    syncPickerFromRgb();
    commandedEffect = null;
    commandedPalette = null;
    commandPending = false;
  }

  onMount(() => {
    listen<DeviceStatus>("device-status", (event) => {
      applyStatus(event.payload);
    });
    listen("device-disconnected", () => {
      connected = false;
      connectedDevice = null;
      connectedIp = "";
      previewPixels = [];
      panels = [{ grid_x: 0, grid_y: 0 }];
      gridWidth = 8;
      gridHeight = 8;
      statusMsg = "Device disconnected";
      stopGif();
    });
    listen<number[]>("preview-frame", (event) => {
      previewPixels = event.payload;
      drawPreview(event.payload);
    });
    // Eagerly load audio output devices
    loadAudioDevices();
  });

  let devices = $state<Device[]>([]);
  let scanning = $state(false);
  let connected = $state(false);
  let connectedIp = $state("");
  let connectedDevice = $state<Device | null>(null);
  let statusMsg = $state("");

  // Control state
  let commandPending = $state(false);
  let commandedEffect = $state<string | null>(null);
  let commandedPalette = $state<string | null>(null);
  let currentEffect = $state("rainbow");
  let brightness = $state(10);
  let sensitivity = $state(50);
  let speed = $state(0.0);
  let direction = $state(0);
  let colorR = $state(0);
  let colorG = $state(0);
  let colorB = $state(255);
  let currentPalette = $state("rainbow");

  // Color set state (3 colors, selectable by index)
  let colorSetColors = $state<[number, number, number][]>([
    [0, 0, 255],
    [255, 0, 0],
    [0, 255, 0],
  ]);
  let selectedColorIndex = $state(0);

  // HSV color picker state
  let pickerHue = $state(240);        // 0-360
  let pickerSaturation = $state(100); // 0-100
  let pickerValue = $state(100);      // 0-100
  let svDragging = $state(false);
  let svPlaneEl: HTMLDivElement | undefined = $state();

  function hsvToRgb(h: number, s: number, v: number): [number, number, number] {
    const s1 = s / 100, v1 = v / 100;
    const c = v1 * s1;
    const x = c * (1 - Math.abs((h / 60) % 2 - 1));
    const m = v1 - c;
    let r1: number, g1: number, b1: number;
    if (h < 60)       { r1 = c; g1 = x; b1 = 0; }
    else if (h < 120) { r1 = x; g1 = c; b1 = 0; }
    else if (h < 180) { r1 = 0; g1 = c; b1 = x; }
    else if (h < 240) { r1 = 0; g1 = x; b1 = c; }
    else if (h < 300) { r1 = x; g1 = 0; b1 = c; }
    else              { r1 = c; g1 = 0; b1 = x; }
    return [Math.round((r1 + m) * 255), Math.round((g1 + m) * 255), Math.round((b1 + m) * 255)];
  }

  function rgbToHsv(r: number, g: number, b: number): [number, number, number] {
    const r1 = r / 255, g1 = g / 255, b1 = b / 255;
    const max = Math.max(r1, g1, b1), min = Math.min(r1, g1, b1);
    const d = max - min;
    let h = 0;
    if (d !== 0) {
      if (max === r1)      h = 60 * (((g1 - b1) / d) % 6);
      else if (max === g1) h = 60 * ((b1 - r1) / d + 2);
      else                 h = 60 * ((r1 - g1) / d + 4);
    }
    if (h < 0) h += 360;
    const s = max === 0 ? 0 : (d / max) * 100;
    const v = max * 100;
    return [h, s, v];
  }

  // Sync picker HSV from RGB when status is received from device
  function syncPickerFromRgb() {
    const [h, s, v] = rgbToHsv(colorR, colorG, colorB);
    pickerHue = Math.round(h);
    pickerSaturation = Math.round(s);
    pickerValue = Math.round(v);
  }

  // Update RGB from picker HSV
  function updateColorFromPicker() {
    const [r, g, b] = hsvToRgb(pickerHue, pickerSaturation, pickerValue);
    colorR = r;
    colorG = g;
    colorB = b;
  }

  function handleSvPointer(e: PointerEvent) {
    if (!svPlaneEl) return;
    const rect = svPlaneEl.getBoundingClientRect();
    const x = Math.max(0, Math.min(e.clientX - rect.left, rect.width));
    const y = Math.max(0, Math.min(e.clientY - rect.top, rect.height));
    pickerSaturation = Math.round((x / rect.width) * 100);
    pickerValue = Math.round((1 - y / rect.height) * 100);
    updateColorFromPicker();
  }

  function onSvPointerDown(e: PointerEvent) {
    svDragging = true;
    svPlaneEl?.setPointerCapture(e.pointerId);
    handleSvPointer(e);
  }

  function onSvPointerMove(e: PointerEvent) {
    if (svDragging) handleSvPointer(e);
  }

  function onSvPointerUp() {
    if (svDragging) {
      svDragging = false;
      setColor();
    }
  }

  function onHueInput() {
    updateColorFromPicker();
  }

  function onHueChange() {
    updateColorFromPicker();
    setColor();
  }

  // Audio state
  interface AudioDevice {
    name: string;
    index: number;
  }

  let audioDevices = $state<AudioDevice[]>([]);
  let selectedDeviceIndex = $state(0);
  let audioStreaming = $state(false);
  let audioStatusMsg = $state("");

  // Grid dimensions and panel topology (from DeviceStatus)
  let gridWidth = $state(8);
  let gridHeight = $state(8);
  let panels = $state<PanelPosition[]>([{ grid_x: 0, grid_y: 0 }]);

  // Live preview state
  let previewPixels = $state<number[]>([]);
  let previewCanvasEl: HTMLCanvasElement | undefined = $state();

  // Image select state
  let imageFilePath = $state("");
  let imagePreviewSrc = $state("");
  let imagePixels = $state<number[]>([]);
  let imageStatusMsg = $state("");
  let imageProcessing = $state(false);

  // GIF playback state
  let gifFrames = $state<{ pixels: number[]; delay_ms: number }[]>([]);
  let gifPlaying = $state(false);
  let gifTimerId = $state<ReturnType<typeof setTimeout> | null>(null);
  let gifFrameIndex = $state(0);

  // WiFi setup state
  let wifiSsid = $state("");
  let wifiPassword = $state("");
  let wifiStatusMsg = $state("");

  // Manual connect state for AP mode
  let manualIp = $state("192.168.4.1");
  let manualPort = $state(5001);

  const effectCategories = [
    { label: "Audio Reactive", effects: ["bass", "splash", "spectrum"] },
    { label: "Single Color",   effects: ["solid", "twinkle", "breathe", "wipe", "matrix", "heartbeat"] },
    { label: "Multi Color",    effects: ["rainbow", "fire", "tunnel", "tetris", "plasma", "sparkle", "glow"] },
    { label: "Image",          effects: ["image", "gif"] },
  ];

  const effects = effectCategories.flatMap(c => c.effects);
  const audioReactiveEffects = new Set(
    effectCategories.find(c => c.label === "Audio Reactive")!.effects
  );

  // Palette definitions — 16 color stops matching firmware (led_math.c)
  const paletteData: Record<string, [number, number, number][]> = {
    rainbow: [
      [255,   0,   0], [255,  96,   0], [255, 191,   0], [191, 255,   0],
      [ 96, 255,   0], [  0, 255,   0], [  0, 255,  96], [  0, 255, 191],
      [  0, 191, 255], [  0,  96, 255], [  0,   0, 255], [ 96,   0, 255],
      [191,   0, 255], [255,   0, 255], [255,   0, 191], [255,   0,  96],
    ],
    fire: [
      [  0,   0,   0], [ 10,   0,   0], [ 30,   0,   0], [ 80,   0,   0],
      [150,   0,   0], [220,  20,   0], [255,  60,   0], [255, 120,   0],
      [255, 180,   0], [255, 220,   0], [255, 255,  20], [255, 255, 100],
      [255, 255, 180], [255, 255, 220], [255, 255, 255], [255, 255, 255],
    ],
    neon: [
      [255,   0, 150], [255,   0,  50], [255,   0,   0], [255,  80,   0],
      [255, 220,   0], [255, 100,   0], [255,   0,   0], [255,   0, 100],
      [255,   0, 255], [150,   0, 255], [ 50,   0, 255], [  0,  60, 255],
      [  0, 180, 255], [  0,  60, 255], [100,   0, 255], [255,   0, 255],
    ],
    ocean: [
      [  0,   0,  40], [  0,   0,  80], [  0,  20, 140], [  0,  60, 200],
      [  0, 100, 255], [  0, 160, 220], [  0, 200, 180], [  0, 220, 140],
      [ 80, 240, 180], [180, 255, 220], [ 80, 240, 180], [  0, 180, 160],
      [  0, 120, 200], [  0,  60, 180], [  0,  20, 100], [  0,   0,  60],
    ],
    sunset: [
      [ 40,   0,  80], [ 80,   0, 120], [140,   0, 140], [200,   0, 100],
      [240,  20,  60], [255,  50,  20], [255, 100,   0], [255, 150,   0],
      [255, 200,  20], [255, 230,  80], [255, 200,  20], [255, 140,   0],
      [255,  60,  10], [200,  10,  60], [120,   0, 120], [ 60,   0, 100],
    ],
    forest: [
      [ 10,  40,   0], [ 20,  80,   0], [  0, 140,  20], [  0, 200,  40],
      [ 20, 255,  40], [ 80, 220,  20], [140, 200,   0], [100, 160,  10],
      [ 60, 100,  10], [ 80,  50,  10], [ 60,  30,   5], [ 40,  60,   5],
      [ 20, 100,  10], [  0, 160,  30], [ 10, 100,  10], [ 10,  60,   0],
    ],
  };

  const palettes = Object.keys(paletteData);

  function paletteGradient(name: string): string {
    const stops = paletteData[name];
    if (!stops) return 'transparent';
    return `linear-gradient(to right, ${stops.map((c, i) => `rgb(${c[0]},${c[1]},${c[2]}) ${(i / 15 * 100).toFixed(1)}%`).join(', ')})`;
  }

  // Effect → panel/parameter visibility config
  const effectConfig: Record<string, {
    palette: boolean;
    color: boolean;
    colorSet: boolean;
    imageSelect: boolean;
    params: ('speed' | 'direction' | 'brightness' | 'sensitivity' | 'audioStream')[];
  }> = {
    rainbow:   { palette: true,  color: false, colorSet: false, imageSelect: false, params: ['brightness', 'speed', 'direction'] },
    bass:      { palette: false, color: true,  colorSet: false, imageSelect: false, params: ['brightness', 'speed', 'sensitivity', 'audioStream'] },
    splash:    { palette: false, color: true,  colorSet: false, imageSelect: false, params: ['brightness', 'speed', 'sensitivity', 'audioStream'] },
    twinkle:   { palette: false, color: true,  colorSet: false, imageSelect: false, params: ['brightness', 'speed'] },
    solid:     { palette: false, color: true,  colorSet: false, imageSelect: false, params: ['brightness'] },
    fire:      { palette: true,  color: false, colorSet: false, imageSelect: false, params: ['brightness'] },
    breathe:   { palette: false, color: true,  colorSet: false, imageSelect: false, params: ['brightness', 'speed'] },
    wipe:      { palette: false, color: true,  colorSet: false, imageSelect: false, params: ['brightness', 'speed'] },
    spectrum:  { palette: true,  color: false, colorSet: false, imageSelect: false, params: ['brightness', 'speed', 'sensitivity', 'audioStream'] },
    image:     { palette: false, color: false, colorSet: false, imageSelect: true,  params: ['brightness'] },
    gif:       { palette: false, color: false, colorSet: false, imageSelect: true,  params: ['brightness'] },
    tunnel:    { palette: false, color: false, colorSet: true,  imageSelect: false, params: ['brightness', 'speed'] },
    tetris:    { palette: false, color: false, colorSet: true,  imageSelect: false, params: ['brightness', 'speed'] },
    plasma:    { palette: true,  color: false, colorSet: false, imageSelect: false, params: ['brightness', 'speed'] },
    sparkle:   { palette: true,  color: false, colorSet: false, imageSelect: false, params: ['brightness', 'speed'] },
    glow:      { palette: true,  color: false, colorSet: false, imageSelect: false, params: ['brightness', 'speed'] },
    matrix:    { palette: false, color: true,  colorSet: false, imageSelect: false, params: ['brightness', 'speed'] },
    heartbeat: { palette: false, color: true,  colorSet: false, imageSelect: false, params: ['brightness', 'speed'] },
  };

  let activeConfig = $derived(effectConfig[currentEffect] ?? effectConfig.rainbow);

  // Derived: available devices (scanned but not currently connected)
  let availableDevices = $derived(
    devices.filter(d => d.ip !== connectedIp)
  );

  // Derived: whether we're connected in AP mode (to the new controller)
  let isApConnection = $derived(connected && connectedIp === manualIp);

  async function scan() {
    scanning = true;
    statusMsg = "Scanning...";
    try {
      devices = await invoke("discover_devices");
      statusMsg = `Found ${availableDevices.length} new device(s)`;
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
      connectedDevice = device;
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
    connectedDevice = null;
    previewPixels = [];
    panels = [{ grid_x: 0, grid_y: 0 }];
    gridWidth = 8;
    gridHeight = 8;
    statusMsg = "Disconnected";
  }

  async function sendCmd(cmd: string): Promise<{ success: boolean; text: string }> {
    try {
      const [success, text]: [boolean, string] = await invoke("send_command", { cmd });
      return { success, text: text.trim() };
    } catch (e) {
      statusMsg = `Error: ${e}`;
      if (String(e).includes("Not connected") || String(e).includes("Connection closed")) {
        connected = false;
        connectedIp = "";
        connectedDevice = null;
      }
      return { success: false, text: "" };
    }
  }

  async function setEffect(name: string) {
    commandPending = true;
    commandedEffect = name;
    await sendCmd(`effect ${name}`);
  }

  async function setBrightness() {
    commandPending = true;
    await sendCmd(`brightness ${brightness}`);
  }

  async function setSensitivity() {
    commandPending = true;
    await sendCmd(`sensitivity ${sensitivity}`);
  }

  async function setSpeed() {
    commandPending = true;
    await sendCmd(`speed ${speed.toFixed(2)}`);
  }

  async function setDirection() {
    commandPending = true;
    await sendCmd(`direction ${direction}`);
  }

  function selectColorSwatch(index: number) {
    selectedColorIndex = index;
    const [r, g, b] = colorSetColors[index];
    colorR = r; colorG = g; colorB = b;
    syncPickerFromRgb();
  }

  async function setColor() {
    commandPending = true;
    if (activeConfig.colorSet) {
      colorSetColors[selectedColorIndex] = [colorR, colorG, colorB];
      colorSetColors = colorSetColors;
      await sendCmd(`color ${selectedColorIndex} ${colorR} ${colorG} ${colorB}`);
    } else {
      colorSetColors[0] = [colorR, colorG, colorB];
      await sendCmd(`color ${colorR} ${colorG} ${colorB}`);
    }
  }

  async function setPalette(name: string) {
    commandPending = true;
    commandedPalette = name;
    await sendCmd(`palette ${name}`);
  }

  // AP mode connection (new controller)
  async function connectManual() {
    try {
      const status: DeviceStatus = await invoke("connect", { ip: manualIp, port: manualPort });
      connected = true;
      connectedIp = manualIp;
      connectedDevice = { name: "New PulseBox", ip: manualIp, cmd_port: manualPort, audio_port: 5000 };
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
    const { text } = await sendCmd(`wifi ${wifiSsid} ${wifiPassword}`);
    wifiStatusMsg = text || "Credentials sent — device is rebooting. Reconnect after it joins your network.";
    connected = false;
    connectedIp = "";
    connectedDevice = null;
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
      loadAudioDevices();
    } catch (e) {
      audioStatusMsg = `Stop failed: ${e}`;
    }
  }

  // Auto-manage audio streaming and GIF playback when switching effects
  let prevEffect: string | null = null;
  $effect(() => {
    const wasAudio = prevEffect !== null && audioReactiveEffects.has(prevEffect);
    const isAudio = audioReactiveEffects.has(currentEffect);

    // Stop GIF playback when leaving gif effect
    if (prevEffect === "gif" && currentEffect !== "gif") {
      stopGif();
    }

    // Clear stale state when switching between image and gif modes
    if ((prevEffect === "gif" && currentEffect === "image") ||
        (prevEffect === "image" && currentEffect === "gif")) {
      imageFilePath = "";
      imagePreviewSrc = "";
      imagePixels = [];
      gifFrames = [];
      imageStatusMsg = "";
    }

    prevEffect = currentEffect;

    if (wasAudio && !isAudio && audioStreaming) {
      stopAudio();
    } else if (!wasAudio && isAudio && connected) {
      loadAudioDevices();
    }
  });

  // Image / GIF select functions
  const isGifMode = $derived(currentEffect === "gif");

  async function chooseImageFile() {
    const filters = isGifMode
      ? [{ name: "GIF", extensions: ["gif"] }]
      : [{ name: "Images", extensions: ["png", "jpg", "jpeg", "bmp", "webp", "tiff", "tif"] }];

    const selected = await open({ multiple: false, filters });
    if (selected) {
      imageFilePath = selected;
      if (isGifMode) {
        await processGif(selected);
      } else {
        await processImage(selected);
      }
    }
  }

  async function processImage(path: string) {
    imageProcessing = true;
    imageStatusMsg = "Processing...";
    gifFrames = [];
    stopGif();
    try {
      const result: { pixels: number[]; preview_png_base64: string } = await invoke(
        "process_image",
        { path, gridWidth, gridHeight }
      );
      imagePixels = result.pixels;
      imagePreviewSrc = `data:image/png;base64,${result.preview_png_base64}`;
      imageStatusMsg = `Ready (${gridWidth}x${gridHeight})`;
    } catch (e) {
      imageStatusMsg = `Error: ${e}`;
      imagePreviewSrc = "";
      imagePixels = [];
    }
    imageProcessing = false;
  }

  async function processGif(path: string) {
    imageProcessing = true;
    imageStatusMsg = "Processing GIF...";
    imagePixels = [];
    stopGif();
    try {
      const result: { frames: { pixels: number[]; delay_ms: number }[]; preview_png_base64: string } =
        await invoke("process_gif", { path, gridWidth, gridHeight });
      gifFrames = result.frames;
      imagePreviewSrc = `data:image/png;base64,${result.preview_png_base64}`;
      imageStatusMsg = `${gifFrames.length} frames (${gridWidth}x${gridHeight})`;
      startGif();
    } catch (e) {
      imageStatusMsg = `Error: ${e}`;
      imagePreviewSrc = "";
      gifFrames = [];
    }
    imageProcessing = false;
  }

  function startGif() {
    if (gifFrames.length === 0) return;
    gifPlaying = true;
    gifFrameIndex = 0;
    sendGifFrame();
  }

  function sendGifFrame() {
    if (!gifPlaying || gifFrames.length === 0) return;
    const frame = gifFrames[gifFrameIndex];
    invoke("send_pixel_frame", { pixels: frame.pixels }).catch(() => {});
    gifFrameIndex = (gifFrameIndex + 1) % gifFrames.length;
    gifTimerId = setTimeout(sendGifFrame, frame.delay_ms);
  }

  function stopGif() {
    gifPlaying = false;
    if (gifTimerId !== null) {
      clearTimeout(gifTimerId);
      gifTimerId = null;
    }
  }

  function drawPreview(pixels: number[]) {
    if (!previewCanvasEl) return;
    const ctx = previewCanvasEl.getContext("2d");
    if (!ctx) return;

    const w = gridWidth;
    const h = gridHeight;
    if (pixels.length !== w * h * 3) return;

    // Compute display size: fit within max 264px wide, maintain aspect ratio, integer scale
    const maxDisplayW = 264;
    const scale = Math.max(1, Math.floor(maxDisplayW / w));
    const displayW = w * scale;
    const displayH = h * scale;
    previewCanvasEl.width = displayW;
    previewCanvasEl.height = displayH;

    // Create ImageData from RGB (expand to RGBA), flipping Y so y=0 is at the bottom
    const imgData = new ImageData(w, h);
    for (let y = 0; y < h; y++) {
      for (let x = 0; x < w; x++) {
        const srcIdx = (y * w + x) * 3;
        const dstIdx = ((h - 1 - y) * w + x) * 4;
        imgData.data[dstIdx + 0] = pixels[srcIdx + 0];
        imgData.data[dstIdx + 1] = pixels[srcIdx + 1];
        imgData.data[dstIdx + 2] = pixels[srcIdx + 2];
        imgData.data[dstIdx + 3] = 255;
      }
    }

    // Draw at native size to an offscreen canvas, then upscale with nearest-neighbor
    const offscreen = new OffscreenCanvas(w, h);
    const offCtx = offscreen.getContext("2d")!;
    offCtx.putImageData(imgData, 0, 0);

    ctx.imageSmoothingEnabled = false;
    ctx.clearRect(0, 0, displayW, displayH);
    ctx.drawImage(offscreen, 0, 0, displayW, displayH);

    // Draw panel boundaries
    ctx.strokeStyle = "rgba(255, 255, 255, 0.3)";
    ctx.lineWidth = 1;
    for (const panel of panels) {
      const px = panel.grid_x * 8 * scale;
      const py = (h - (panel.grid_y + 1) * 8) * scale; // flip Y for screen coords
      const pw = 8 * scale;
      const ph = 8 * scale;
      ctx.strokeRect(px + 0.5, py + 0.5, pw - 1, ph - 1);
    }
  }

  async function uploadImage() {
    if (imagePixels.length === 0) return;
    try {
      await invoke("send_pixel_frame", { pixels: imagePixels });
      imageStatusMsg = "Uploaded!";
    } catch (e) {
      imageStatusMsg = `Upload failed: ${e}`;
    }
  }
</script>

<div class="app">
  <!-- Banner -->
  <header class="banner">
    <h1>Pulse Box</h1>
    <img src="/pulsebox_logo.svg" alt="PulseBox Logo" class="banner-logo" />
  </header>

  <!-- 3-Column Grid -->
  <div class="grid">
    <!-- Column 1: Controllers + Panel Orientation -->
    <div class="column">
      <!-- Controllers Panel -->
      <section class="panel controllers-panel">
        <h2 class="panel-header">CONTROLLERS</h2>

        <!-- Connected Devices -->
        <div class="sub-panel">
          <div class="sub-header">CONNECTED</div>
          {#if connectedDevice}
            <div class="device-row">
              <span class="device-info">Name: {connectedDevice.name} &nbsp;|&nbsp; IP: {connectedIp}</span>
              <button class="pill-btn pill-warning" onclick={disconnectDevice}>Disconnect</button>
            </div>
          {:else}
            <span class="none-text">None</span>
          {/if}
        </div>

        <!-- Available Devices -->
        <div class="sub-panel">
          <div class="sub-header">AVAILABLE</div>
          {#if availableDevices.length > 0}
            {#each availableDevices as device}
              <div class="device-row">
                <span class="device-info">Name: {device.name} &nbsp;|&nbsp; IP: {device.ip}</span>
                <button class="pill-btn pill-success" onclick={() => connectDevice(device)}>Connect</button>
              </div>
            {/each}
          {:else}
            <span class="none-text">None</span>
          {/if}
        </div>

        <!-- Scan Button -->
        <button class="pill-btn pill-accent scan-btn" onclick={scan} disabled={scanning}>
          {scanning ? "Scanning..." : "Scan For New Controllers"}
        </button>

        {#if statusMsg}
          <div class="status-msg">{statusMsg}</div>
        {/if}

        <!-- Hint -->
        <p class="hint">For first-time setup of a new controller, connect your PC to the "PulseBox-Setup" WiFi network (password: pulsebox123), then click connect below</p>

        <!-- New Controller Configuration -->
        <div class="sub-panel">
          <div class="sub-header">NEW CONTROLLER CONFIGURATION</div>
          {#if isApConnection}
            <div class="device-row">
              <span class="device-info">Name: New PulseBox</span>
              <button class="pill-btn pill-success" disabled>Connected</button>
            </div>
          {:else}
            <div class="device-row">
              <span class="device-info">Name: New PulseBox</span>
              <button class="pill-btn pill-success" onclick={connectManual}>Connect</button>
            </div>
          {/if}
          <div class="wifi-row">
            <input type="text" class="wifi-input" bind:value={wifiSsid} placeholder="WiFi SSID" disabled={!isApConnection} />
            <input type="password" class="wifi-input" bind:value={wifiPassword} placeholder="WiFi Password" disabled={!isApConnection} />
            <button class="pill-btn pill-accent" onclick={configureWifi} disabled={!isApConnection}>Save &amp; Reboot</button>
          </div>
          <p class="hint">Upon connecting, enter WiFi credentials, then Save &amp; Reboot. Controller will be available on next scan after reboot.</p>
          {#if wifiStatusMsg}
            <div class="status-msg">{wifiStatusMsg}</div>
          {/if}
        </div>
      </section>

      <!-- Panel Preview -->
      <section class="panel preview-panel">
        <h2 class="panel-header">PANEL PREVIEW</h2>
        <div class="grid-preview-container">
          {#if connected}
            <div class="preview-canvas-wrapper">
              <canvas
                bind:this={previewCanvasEl}
                class="preview-canvas"
              ></canvas>
            </div>
            <div class="grid-info">
              {panels.length} panel{panels.length !== 1 ? "s" : ""} &middot; {gridWidth}&times;{gridHeight} px
            </div>
          {:else}
            <div class="preview-disconnected">
              <span class="tile-pos dim">No device connected</span>
            </div>
          {/if}
        </div>
      </section>
    </div>

    <!-- Column 2: Effects + Dynamic Parameters -->
    <div class="column">
      <!-- Effects Panel -->
      <section class="panel effects-panel">
        <h2 class="panel-header">EFFECT</h2>
        {#each effectCategories as category}
          <div class="effect-category">
            <div class="sub-header">{category.label}</div>
            <div class="effect-grid">
              {#each category.effects as effect}
                <button
                  class="effect-btn"
                  class:active={currentEffect === effect}
                  class:commanded={commandedEffect === effect}
                  disabled={!connected || commandPending}
                  onclick={() => setEffect(effect)}
                >
                  {effect}
                </button>
              {/each}
            </div>
          </div>
        {/each}
      </section>

      {#if activeConfig.palette}
        <section class="panel">
          <h2 class="panel-header">PALETTE</h2>
          <div class="param-grid">
            {#each palettes as palette}
              <button
                class="palette-name-btn"
                class:active={currentPalette === palette}
                class:commanded={commandedPalette === palette}
                disabled={!connected || commandPending}
                onclick={() => setPalette(palette)}
              >
                {palette}
              </button>
              <div
                class="palette-preview"
                class:active={currentPalette === palette}
                style="background: {paletteGradient(palette)}"
              ></div>
            {/each}
          </div>
        </section>
      {/if}

      {#if activeConfig.colorSet}
        <section class="panel">
          <h2 class="panel-header">COLOR SET</h2>
          <div class="color-picker">
            <div class="color-picker-row">
              <div class="color-set-swatches">
                {#each colorSetColors as [r, g, b], i}
                  <button
                    class="color-set-swatch"
                    class:selected={selectedColorIndex === i}
                    style="background: rgb({r},{g},{b})"
                    onclick={() => selectColorSwatch(i)}
                    disabled={!connected || commandPending}
                    aria-label="Color {i + 1}"
                  >
                    {i + 1}
                  </button>
                {/each}
              </div>
              <div
                class="sv-plane"
                bind:this={svPlaneEl}
                style="background-color: hsl({pickerHue}, 100%, 50%)"
                onpointerdown={onSvPointerDown}
                onpointermove={onSvPointerMove}
                onpointerup={onSvPointerUp}
                role="slider"
                tabindex="0"
                aria-label="Saturation and brightness"
              >
                <div class="sv-white"></div>
                <div class="sv-black"></div>
                <div
                  class="sv-cursor"
                  style="left: {pickerSaturation}%; top: {100 - pickerValue}%"
                ></div>
              </div>
            </div>
            <input
              type="range"
              class="hue-slider"
              min="0"
              max="360"
              bind:value={pickerHue}
              oninput={onHueInput}
              onchange={onHueChange}
              disabled={!connected || commandPending}
            />
            <div class="color-readout">
              <span class="param-label">R: {colorR}</span>
              <span class="param-label">G: {colorG}</span>
              <span class="param-label">B: {colorB}</span>
            </div>
          </div>
        </section>
      {/if}

      {#if activeConfig.color}
        <section class="panel">
          <h2 class="panel-header">COLOR</h2>
          <div class="color-picker">
            <div class="color-picker-row">
              <!-- Current color swatch -->
              <div class="color-swatch" style="background: rgb({colorR},{colorG},{colorB})"></div>
              <!-- SV Plane -->
              <div
                class="sv-plane"
                bind:this={svPlaneEl}
                style="background-color: hsl({pickerHue}, 100%, 50%)"
                onpointerdown={onSvPointerDown}
                onpointermove={onSvPointerMove}
                onpointerup={onSvPointerUp}
                role="slider"
                tabindex="0"
                aria-label="Saturation and brightness"
              >
                <div class="sv-white"></div>
                <div class="sv-black"></div>
                <div
                  class="sv-cursor"
                  style="left: {pickerSaturation}%; top: {100 - pickerValue}%"
                ></div>
              </div>
            </div>
            <!-- Hue Slider -->
            <input
              type="range"
              class="hue-slider"
              min="0"
              max="360"
              bind:value={pickerHue}
              oninput={onHueInput}
              onchange={onHueChange}
              disabled={!connected || commandPending}
            />
            <!-- RGB readout -->
            <div class="color-readout">
              <span class="param-label">R: {colorR}</span>
              <span class="param-label">G: {colorG}</span>
              <span class="param-label">B: {colorB}</span>
            </div>
          </div>
        </section>
      {/if}

      <section class="panel">
        <h2 class="panel-header">PARAMETERS</h2>
        <div class="param-grid">
          {#if activeConfig.params.includes('brightness')}
            <span class="param-label">Brightness (0-100%)</span>
            <div class="param-control">
              <input type="range" class="param-slider" min="0" max="100" bind:value={brightness} onchange={setBrightness} disabled={!connected || commandPending} />
              <input type="number" class="param-input" min="0" max="100" bind:value={brightness} onchange={setBrightness} disabled={!connected || commandPending} />
            </div>
          {/if}

          {#if activeConfig.params.includes('sensitivity')}
            <span class="param-label">Sensitivity (0-100)</span>
            <div class="param-control">
              <input type="range" class="param-slider" min="0" max="100" bind:value={sensitivity} onchange={setSensitivity} disabled={!connected || commandPending} />
              <input type="number" class="param-input" min="0" max="100" bind:value={sensitivity} onchange={setSensitivity} disabled={!connected || commandPending} />
            </div>
          {/if}

          {#if activeConfig.params.includes('speed')}
            <span class="param-label">Speed (0.00-1.00)</span>
            <div class="param-control">
              <input type="range" class="param-slider" min="0" max="1" step="0.01" bind:value={speed} onchange={setSpeed} disabled={!connected || commandPending} />
              <input type="number" class="param-input" min="0" max="1" step="0.01" bind:value={speed} onchange={setSpeed} disabled={!connected || commandPending} />
            </div>
          {/if}

          {#if activeConfig.params.includes('direction')}
            <span class="param-label">Direction (0-360&deg;)</span>
            <div class="param-control">
              <input type="range" class="param-slider" min="0" max="360" bind:value={direction} onchange={setDirection} disabled={!connected || commandPending} />
              <input type="number" class="param-input" min="0" max="360" bind:value={direction} onchange={setDirection} disabled={!connected || commandPending} />
            </div>
          {/if}

          {#if activeConfig.params.includes('audioStream')}
            <span class="param-label">Audio Stream</span>
            <div class="param-control audio-stream-control">
              {#if audioStreaming}
                <span class="streaming-indicator">Streaming</span>
                <button class="pill-btn pill-warning" onclick={stopAudio}>Stop</button>
              {:else}
                <select class="param-select" bind:value={selectedDeviceIndex} disabled={!connected || commandPending}>
                  {#each audioDevices as device}
                    <option value={device.index}>{device.name}</option>
                  {/each}
                  {#if audioDevices.length === 0}
                    <option disabled>No devices found</option>
                  {/if}
                </select>
                <button class="pill-btn pill-accent" onclick={startAudio} disabled={!connected || audioDevices.length === 0}>Stream</button>
              {/if}
            </div>
          {/if}
        </div>
        {#if audioStatusMsg && activeConfig.params.includes('audioStream')}
          <div class="status-msg">{audioStatusMsg}</div>
        {/if}
      </section>

      {#if activeConfig.imageSelect}
        <section class="panel">
          <div class="image-select-grid">
            <!-- Left column: controls -->
            <div class="image-select-controls">
              <h2 class="panel-header">{isGifMode ? 'GIF SELECT' : 'IMAGE SELECT'}</h2>
              <div class="image-file-row">
                <button
                  class="pill-btn pill-accent image-choose-btn"
                  onclick={chooseImageFile}
                  disabled={!connected || imageProcessing}
                >
                  Choose File
                </button>
                <span class="image-file-path">
                  {imageFilePath ? imageFilePath.split(/[\\/]/).pop() : "No file chosen"}
                </span>
              </div>
              {#if isGifMode}
                <div class="gif-controls">
                  {#if gifPlaying}
                    <button
                      class="pill-btn pill-warning image-upload-btn"
                      onclick={stopGif}
                    >
                      Pause
                    </button>
                  {:else}
                    <button
                      class="pill-btn pill-accent image-upload-btn"
                      onclick={startGif}
                      disabled={!connected || gifFrames.length === 0}
                    >
                      Play
                    </button>
                  {/if}
                </div>
              {:else}
                <button
                  class="pill-btn pill-accent image-upload-btn"
                  onclick={uploadImage}
                  disabled={!connected || imagePixels.length === 0 || imageProcessing}
                >
                  Upload
                </button>
              {/if}
              {#if imageStatusMsg}
                <div class="status-msg">{imageStatusMsg}</div>
              {/if}
            </div>
            <!-- Right column: preview -->
            <div class="image-preview-col">
              <span class="image-preview-label">Preview</span>
              <div class="image-preview-box">
                {#if imagePreviewSrc}
                  <img
                    src={imagePreviewSrc}
                    alt="Image preview"
                    class="image-preview-img"
                  />
                {/if}
              </div>
            </div>
          </div>
        </section>
      {/if}
    </div>

  </div>
</div>

<style>
  @import url('https://fonts.googleapis.com/css2?family=Audiowide&display=swap');

  :root {
    /* Theme palette */
    --color-bg: #1a1a1a;
    --color-surface: #2a2a2a;
    --color-elevated: #2f2f2f;
    --color-elevated-hover: #383838;
    --color-border: #444444;
    --color-text: #e0e0e0;
    --color-text-secondary: #999999;
    --color-text-dim: #666666;
    --color-accent: #8faabe;
    --color-accent-active: #7aa3c4;

    /* Semantic colors */
    --color-success: #4ade80;
    --color-warning: #f59e0b;
    --color-commanded: #b45309;

    font-family: Inter, system-ui, sans-serif;
    font-size: 14px;
    color: var(--color-text);
    background-color: var(--color-bg);
  }

  :global(html, body) {
    margin: 0;
    padding: 0;
    height: 100%;
    overflow: hidden;
  }

  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }

  /* Banner */
  .banner {
    padding: 16px 24px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 16px;
  }

  .banner h1 {
    font-family: 'Audiowide', sans-serif;
    font-size: 48px;
    margin: 0;
    color: var(--color-text);
  }

  .banner-logo {
    width: 64px;
    height: 64px;
  }

  /* Grid layout */
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
    flex: 1;
    padding: 0 24px 24px;
    overflow: hidden;
  }

  .column {
    display: flex;
    flex-direction: column;
    gap: 12px;
    overflow-y: auto;
    min-height: 0;
  }

  /* Panels */
  .panel {
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 10px;
    padding: 15px;
  }

  .panel-header {
    font-family: Arial, sans-serif;
    font-weight: 700;
    font-size: 24px;
    color: var(--color-text-secondary);
    margin: 0 0 11px 0;
    text-transform: uppercase;
  }

  .preview-panel {
    display: flex;
    flex-direction: column;
    width: fit-content;
  }

  .grid-preview-container {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .preview-canvas-wrapper {
    display: flex;
    justify-content: center;
    align-items: center;
    padding: 8px;
    background: var(--color-bg);
    border-radius: 6px;
    border: 1px solid var(--color-border);
  }

  .preview-canvas {
    image-rendering: pixelated;
    border-radius: 4px;
  }

  .grid-info {
    font-family: Arial, sans-serif;
    font-size: 12px;
    color: var(--color-text-secondary);
    text-align: center;
  }

  .preview-disconnected {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 80px;
    background: var(--color-bg);
    border-radius: 6px;
    border: 1px dashed var(--color-border);
  }

  .tile-pos.dim {
    font-family: Arial, sans-serif;
    font-size: 14px;
    color: var(--color-text-dim);
  }

  /* Controllers panel sub-panels */
  .controllers-panel {
    display: flex;
    flex-direction: column;
    gap: 11px;
  }

  .sub-panel {
    background: var(--color-elevated);
    border: 1px solid var(--color-border);
    border-radius: 10px;
    padding: 5px 10px;
    display: flex;
    flex-direction: column;
    gap: 7px;
  }

  .sub-header {
    font-family: Arial, sans-serif;
    font-weight: 400;
    font-size: 16px;
    color: var(--color-text);
  }

  .device-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 7px;
  }

  .device-info {
    font-family: Arial, sans-serif;
    font-size: 12px;
    color: white;
  }

  .none-text {
    font-family: Arial, sans-serif;
    font-size: 12px;
    color: var(--color-text-dim);
  }

  /* Pill buttons (Connect, Disconnect, Scan, Save & Reboot) */
  .pill-btn {
    font-family: Arial, sans-serif;
    font-size: 12px;
    border: 1px solid var(--color-border);
    border-radius: 10px;
    padding: 2px 16px;
    cursor: pointer;
    white-space: nowrap;
    height: 16px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .pill-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .pill-success {
    background: var(--color-success);
    color: var(--color-elevated-hover);
  }

  .pill-success:hover:not(:disabled) {
    opacity: 0.85;
  }

  .pill-warning {
    background: var(--color-warning);
    color: var(--color-elevated-hover);
  }

  .pill-warning:hover:not(:disabled) {
    opacity: 0.85;
  }

  .pill-accent {
    background: var(--color-accent);
    color: white;
  }

  .pill-accent:hover:not(:disabled) {
    background: var(--color-accent-active);
  }

  .scan-btn {
    height: 30px;
    width: fit-content;
    padding: 0 16px;
    font-size: 12px;
  }

  /* WiFi config row */
  .wifi-row {
    display: flex;
    gap: 6px;
    align-items: center;
  }

  .wifi-input {
    background: var(--color-elevated-hover);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: 5px;
    padding: 4px 8px;
    font-family: Arial, sans-serif;
    font-size: 12px;
    width: 137px;
  }

  .wifi-input::placeholder {
    color: var(--color-text-dim);
  }

  .wifi-input:disabled {
    opacity: 0.5;
  }

  /* Status and hints */
  .status-msg {
    font-size: 12px;
    color: var(--color-text-secondary);
  }

  .hint {
    font-family: Arial, sans-serif;
    font-size: 10px;
    color: var(--color-text-dim);
    margin: 0;
    line-height: 1.3;
  }

  /* Effects panel */
  .effects-panel {
    display: flex;
    flex-direction: column;
    gap: 11px;
  }

  .effect-category {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  /* Effect grid */
  .effect-grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 6px;
  }

  .effect-btn {
    background: var(--color-elevated);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: 6px;
    padding: 10px;
    cursor: pointer;
    font-size: 12px;
    text-transform: capitalize;
  }

  .effect-btn:hover:not(:disabled) {
    background: var(--color-elevated-hover);
  }

  .effect-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .effect-btn.active {
    background: var(--color-accent);
    border-color: var(--color-accent-active);
  }

  .effect-btn.commanded {
    background: var(--color-commanded);
    color: var(--color-text-secondary);
    opacity: 0.7;
  }

  /* Color picker */
  .color-picker {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .color-picker-row {
    display: flex;
    gap: 10px;
    align-items: stretch;
  }

  .color-swatch {
    width: 64px;
    flex-shrink: 0;
    border-radius: 6px;
    border: 2px solid var(--color-border);
  }

  .color-set-swatches {
    display: flex;
    flex-direction: column;
    gap: 6px;
    flex-shrink: 0;
  }

  .color-set-swatch {
    width: 48px;
    height: 48px;
    border-radius: 6px;
    border: 2px solid var(--color-border);
    cursor: pointer;
    font-size: 14px;
    font-weight: 700;
    color: white;
    text-shadow: 0 0 3px rgba(0, 0, 0, 0.8);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .color-set-swatch.selected {
    border-color: white;
    box-shadow: 0 0 6px rgba(255, 255, 255, 0.4);
  }

  .color-set-swatch:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .sv-plane {
    position: relative;
    flex: 1;
    height: 160px;
    border-radius: 6px;
    cursor: crosshair;
    touch-action: none;
    overflow: hidden;
  }

  .sv-white {
    position: absolute;
    inset: 0;
    background: linear-gradient(to right, white, transparent);
  }

  .sv-black {
    position: absolute;
    inset: 0;
    background: linear-gradient(to bottom, transparent, black);
  }

  .sv-cursor {
    position: absolute;
    width: 12px;
    height: 12px;
    border: 2px solid white;
    border-radius: 50%;
    box-shadow: 0 0 2px rgba(0, 0, 0, 0.6);
    transform: translate(-50%, -50%);
    pointer-events: none;
  }

  .hue-slider {
    width: 100%;
    height: 16px;
    -webkit-appearance: none;
    appearance: none;
    border-radius: 8px;
    background: linear-gradient(to right,
      hsl(0, 100%, 50%),
      hsl(60, 100%, 50%),
      hsl(120, 100%, 50%),
      hsl(180, 100%, 50%),
      hsl(240, 100%, 50%),
      hsl(300, 100%, 50%),
      hsl(360, 100%, 50%)
    );
    outline: none;
    cursor: pointer;
  }

  .hue-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 8px;
    height: 20px;
    border-radius: 3px;
    background: white;
    border: 1px solid var(--color-border);
    box-shadow: 0 0 3px rgba(0, 0, 0, 0.4);
    cursor: pointer;
  }

  .hue-slider::-moz-range-thumb {
    width: 8px;
    height: 20px;
    border-radius: 3px;
    background: white;
    border: 1px solid var(--color-border);
    box-shadow: 0 0 3px rgba(0, 0, 0, 0.4);
    cursor: pointer;
  }

  .hue-slider:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .color-readout {
    display: flex;
    gap: 16px;
  }

  /* Palette panel */
  .palette-name-btn {
    background: var(--color-elevated);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: 6px;
    padding: 8px 12px;
    cursor: pointer;
    font-family: Arial, sans-serif;
    font-size: 14px;
    text-transform: capitalize;
    text-align: center;
  }

  .palette-name-btn:hover:not(:disabled) {
    background: var(--color-elevated-hover);
  }

  .palette-name-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .palette-name-btn.active {
    background: var(--color-accent);
    border-color: var(--color-accent-active);
  }

  .palette-name-btn.commanded {
    background: var(--color-commanded);
    color: var(--color-text-secondary);
    opacity: 0.7;
  }

  .palette-preview {
    height: 32px;
    border-radius: 6px;
    border: 1px solid var(--color-border);
    opacity: 0.6;
  }

  .palette-preview.active {
    opacity: 1;
    border-color: var(--color-accent-active);
  }

  /* Parameter grid */
  .param-grid {
    display: grid;
    grid-template-columns: 1fr 2fr;
    gap: 10px 12px;
    align-items: center;
  }

  .param-label {
    font-family: Arial, sans-serif;
    font-size: 14px;
    color: var(--color-text);
  }

  .param-control {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .param-slider {
    flex: 1;
    accent-color: var(--color-accent);
  }

  .param-slider:disabled {
    opacity: 0.4;
  }

  .param-input {
    width: 56px;
    background: var(--color-elevated-hover);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: 5px;
    padding: 4px 6px;
    font-family: Arial, sans-serif;
    font-size: 12px;
    text-align: center;
    -moz-appearance: textfield;
  }

  .param-input::-webkit-outer-spin-button,
  .param-input::-webkit-inner-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }

  .param-input:disabled {
    opacity: 0.4;
  }

  .param-unit {
    font-family: Arial, sans-serif;
    font-size: 12px;
    color: var(--color-text-secondary);
    width: 16px;
  }

  .param-select {
    flex: 1;
    background: var(--color-elevated-hover);
    color: var(--color-text);
    border: 1px solid var(--color-border);
    border-radius: 5px;
    padding: 4px 8px;
    font-family: Arial, sans-serif;
    font-size: 12px;
    min-width: 0;
  }

  .param-select:disabled {
    opacity: 0.4;
  }

  .audio-stream-control {
    min-width: 0;
  }

  .streaming-indicator {
    font-family: Arial, sans-serif;
    font-size: 12px;
    color: var(--color-warning);
    flex: 1;
  }

  /* Image Select panel */
  .image-select-grid {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 11px;
    align-items: start;
  }

  .image-select-controls {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .image-select-controls .panel-header {
    margin-bottom: 0;
  }

  .image-file-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .image-choose-btn {
    height: 24px;
    padding: 0 14px;
    flex-shrink: 0;
  }

  .image-file-path {
    font-family: Arial, sans-serif;
    font-size: 12px;
    color: var(--color-text);
    background: var(--color-elevated-hover);
    border: 1px solid var(--color-border);
    border-radius: 5px;
    padding: 4px 8px;
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .image-upload-btn {
    height: 24px;
    padding: 0 24px;
    width: fit-content;
  }

  .image-preview-col {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .image-preview-label {
    font-family: Arial, sans-serif;
    font-weight: 700;
    font-size: 24px;
    color: white;
  }

  .image-preview-box {
    width: 128px;
    height: 128px;
    background: var(--color-elevated);
    border: 1px solid var(--color-border);
    border-radius: 10px;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
  }

  .image-preview-img {
    width: 128px;
    height: 128px;
    image-rendering: pixelated;
    border-radius: 10px;
  }
</style>
