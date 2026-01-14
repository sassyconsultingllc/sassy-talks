# 🖥️ SassyTalkie Desktop - Complete Implementation Guide
## Windows / Mac / Linux Cross-Platform

---

## ✅ WHAT'S ALREADY DONE

### Infrastructure Complete
- ✅ Tauri 2.0 setup with React frontend
- ✅ Cargo.toml with all dependencies
- ✅ Cross-platform audio (cpal)
- ✅ Opus codec support
- ✅ AES-256-GCM encryption
- ✅ UDP multicast transport
- ✅ Module structure in place

### Files Completed
- ✅ **audio.rs** (JUST BUILT - 400 lines)
  - Cross-platform audio I/O with cpal
  - Ring buffer implementation
  - Input/output device selection
  - Volume control
  - Recording/playback state management

---

## 🔨 WHAT NEEDS TO BE BUILT (Remaining 80%)

### Priority 1: Core Communication (CRITICAL)

#### 1. codec.rs - Opus Encoding/Decoding
```rust
// Already has audiopus dependency
// Need to implement:
pub struct OpusEncoder {
    encoder: audiopus::coder::Encoder,
    frame_size: usize,
}

pub struct OpusDecoder {
    decoder: audiopus::coder::Decoder,
    frame_size: usize,
}

// Key methods:
- encode(&mut self, pcm: &[i16]) -> Result<Vec<u8>>
- decode(&mut self, opus: &[u8]) -> Result<Vec<i16>>
```

**Estimated:** 200 lines, 2 hours

#### 2. transport/mod.rs - UDP Multicast
```rust
// Use socket2 for multicast UDP
// This works on all platforms (WiFi required)

pub struct TransportManager {
    socket: Arc<Mutex<UdpSocket>>,
    multicast_addr: SocketAddr, // e.g., 239.255.42.42:5555
    peers: Arc<RwLock<HashMap<u32, PeerInfo>>>,
}

// Key methods:
- start_discovery() // Send beacon packets
- send_audio(channel: u8, data: &[u8])
- receive_loop() // Background thread
- get_peers() -> Vec<PeerInfo>
```

**Estimated:** 300 lines, 3 hours

#### 3. protocol.rs - Packet Format
```rust
// Already structured, needs implementation

pub enum PacketType {
    Discovery,      // Peer announces presence
    Audio,          // Voice data
    KeepAlive,     // Maintain connection
}

pub struct Packet {
    pub device_id: u32,
    pub channel: u8,
    pub packet_type: PacketType,
    pub payload: Vec<u8>,
    pub checksum: u32,
}

// Methods:
- serialize() -> Vec<u8>
- deserialize(bytes: &[u8]) -> Result<Packet>
```

**Estimated:** 150 lines, 1 hour

---

### Priority 2: State Management (CRITICAL)

#### 4. lib.rs - AppState
```rust
pub struct AppState {
    // Core engines
    audio: Arc<Mutex<AudioEngine>>,
    codec_encoder: Arc<Mutex<OpusEncoder>>,
    codec_decoder: Arc<Mutex<OpusDecoder>>,
    transport: Arc<Mutex<TransportManager>>,
    crypto: Arc<Mutex<CryptoEngine>>,
    
    // State
    device_id: u32,
    device_name: String,
    current_channel: Arc<AtomicU8>,
    is_transmitting: Arc<AtomicBool>,
    is_connected: Arc<AtomicBool>,
    
    // PTT thread
    tx_thread: Arc<Mutex<Option<JoinHandle<()>>>>,
    rx_thread: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl AppState {
    pub fn new(device_id: u32, device_name: String) -> Self { }
    
    // PTT lifecycle
    pub fn start_transmit(&self) -> Result<()> {
        // 1. Start audio recording
        // 2. Spawn TX thread:
        //    - Read samples from audio engine
        //    - Encode with Opus
        //    - Encrypt with AES
        //    - Send via UDP multicast
    }
    
    pub fn stop_transmit(&self) -> Result<()> { }
    
    pub fn start_receiving(&self) -> Result<()> {
        // Spawn RX thread:
        //    - Receive UDP packets
        //    - Decrypt
        //    - Decode Opus
        //    - Write to audio engine
    }
}
```

**Estimated:** 400 lines, 4 hours

---

### Priority 3: Tauri Commands (CRITICAL FOR UI)

#### 5. commands.rs - Frontend API
```rust
// Already has function signatures in main.rs
// Need full implementations:

#[tauri::command]
pub async fn start_transmit(
    state: State<'_, Arc<AppState>>
) -> Result<(), String> {
    state.start_transmit()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn stop_transmit(
    state: State<'_, Arc<AppState>>
) -> Result<(), String> {
    state.stop_transmit()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_nearby_devices(
    state: State<'_, Arc<AppState>>
) -> Result<Vec<PeerInfo>, String> {
    Ok(state.get_peers())
}

#[tauri::command]
pub async fn get_audio_devices(
    state: State<'_, Arc<AppState>>
) -> Result<AudioDevices, String> {
    // Return { inputs: [...], outputs: [...] }
}

// Plus 15 more commands already defined in main.rs
```

**Estimated:** 300 lines, 2 hours

---

### Priority 4: Frontend UI (CRITICAL)

#### 6. React UI - src/App.tsx
```typescript
// Complete PTT interface similar to Android

interface AppState {
  channel: number;
  isTransmitting: boolean;
  isReceiving: boolean;
  nearbyDevices: PeerInfo[];
  connectionStatus: string;
}

function App() {
  // State management
  const [channel, setChannel] = useState(1);
  const [isTransmitting, setIsTransmitting] = useState(false);
  const [devices, setDevices] = useState<PeerInfo[]>([]);
  
  // PTT button handler
  const handlePTTPress = async () => {
    await invoke('start_transmit');
    setIsTransmitting(true);
  };
  
  const handlePTTRelease = async () => {
    await invoke('stop_transmit');
    setIsTransmitting(false);
  };
  
  // Render UI similar to Android version
  return (
    <div className="app">
      <ChannelSelector channel={channel} onChange={setChannel} />
      <PTTButton 
        onMouseDown={handlePTTPress}
        onMouseUp={handlePTTRelease}
        onTouchStart={handlePTTPress}
        onTouchEnd={handlePTTRelease}
        isTransmitting={isTransmitting}
      />
      <DeviceList devices={devices} />
      <StatusBar status={status} />
    </div>
  );
}
```

**Estimated:** 800 lines (TSX + CSS), 5 hours

---

## 🎯 DESKTOP-SPECIFIC ARCHITECTURE

### Transport Strategy: UDP Multicast (WiFi)

**Why UDP Multicast?**
- ✅ Works on Windows/Mac/Linux
- ✅ No pairing required
- ✅ Automatic peer discovery
- ✅ Low latency (< 100ms)
- ✅ Supports multiple peers (group calls)

**How It Works:**
```
Device 1 (192.168.1.100) ─┐
Device 2 (192.168.1.101) ─┼─> Multicast Group 239.255.42.42:5555
Device 3 (192.168.1.102) ─┘

All devices on same WiFi network automatically discover each other
```

### Audio Pipeline
```
[Microphone]
    ↓
[CPAL AudioEngine] - Platform-specific audio APIs
    ↓
[Ring Buffer] - 960 samples (20ms)
    ↓
[Opus Encoder] - Compress 960 i16 → ~60 bytes
    ↓
[AES-256-GCM] - Encrypt
    ↓
[UDP Multicast] - Send to 239.255.42.42:5555
    ↓
[All Peers on Channel X]
    ↓
[Decrypt] - AES-256-GCM
    ↓
[Opus Decoder] - Decompress
    ↓
[CPAL AudioEngine]
    ↓
[Speaker]
```

---

## 📋 COMPLETION CHECKLIST

### Backend (Rust)
- [x] audio.rs - Cross-platform audio I/O
- [ ] codec.rs - Opus encoding/decoding
- [ ] transport/mod.rs - UDP multicast
- [ ] transport/discovery.rs - Peer discovery
- [ ] protocol.rs - Packet format
- [ ] lib.rs - AppState implementation
- [ ] commands.rs - All 20 Tauri commands
- [ ] security/mod.rs - AES encryption integration

### Frontend (React/TypeScript)
- [ ] App.tsx - Main component
- [ ] components/PTTButton.tsx
- [ ] components/ChannelSelector.tsx
- [ ] components/DeviceList.tsx
- [ ] components/StatusBar.tsx
- [ ] components/SettingsPanel.tsx
- [ ] styles/app.css - Retro theme
- [ ] hooks/useAudio.ts
- [ ] hooks/useDevices.ts

### Testing
- [ ] Local testing (same computer)
- [ ] LAN testing (2 computers, same WiFi)
- [ ] Windows 10/11 testing
- [ ] macOS testing
- [ ] Linux (Ubuntu) testing
- [ ] Audio quality verification
- [ ] Latency testing (< 200ms target)

---

## ⏱️ TIME ESTIMATE

| Task | Hours |
|------|-------|
| codec.rs | 2 |
| transport.rs | 3 |
| protocol.rs | 1 |
| lib.rs (AppState) | 4 |
| commands.rs | 2 |
| React UI | 5 |
| Testing & Polish | 3 |
| **TOTAL** | **20 hours** |

---

## 🚀 QUICK START GUIDE (Once Complete)

### Build
```bash
cd tauri-desktop
npm install
npm run tauri build
```

### Run Dev
```bash
npm run tauri dev
```

### Test
```bash
# Terminal 1
npm run tauri dev

# Terminal 2 (on another computer)
npm run tauri dev

# Both on same WiFi network
# Should auto-discover each other
# Press PTT to talk!
```

---

## 🎨 UI DESIGN (Retro Theme)

### Color Scheme
```css
:root {
  --dark-bg: #1a1a2e;
  --card-bg: #252540;
  --orange: #ff8c00;
  --cyan: #00e6c8;
  --green: #4cd964;
  --red: #ef5350;
}
```

### Layout
```
┌─────────────────────────┐
│  🎙️ Sassy-Talk Desktop │
├─────────────────────────┤
│   Channel:  [< 01 >]    │
├─────────────────────────┤
│                         │
│       ┌─────────┐       │
│       │         │       │
│       │   PTT   │       │
│       │         │       │
│       └─────────┘       │
│                         │
├─────────────────────────┤
│ 📡 Nearby Devices (3)   │
│  • Windows PC (192...)  │
│  • MacBook Pro          │
│  • Linux Desktop        │
├─────────────────────────┤
│ Status: Connected ✓     │
└─────────────────────────┘
```

---

## 🔐 SECURITY NOTES

### Encryption
- AES-256-GCM for all audio packets
- Pre-shared key (Channel # = Key seed)
- Future: X25519 key exchange for dynamic keys

### Privacy
- No internet connection required
- Local network only
- No data collection
- No servers

---

## 🎯 PRODUCTION READINESS

### What Works Now
- ✅ Cross-platform audio I/O
- ✅ Tauri framework setup
- ✅ Dependencies configured
- ✅ Module structure

### What's Needed
- ❌ UDP multicast implementation
- ❌ Opus codec integration
- ❌ State machine
- ❌ React UI
- ❌ Peer discovery
- ❌ Testing on all platforms

### Status
**20% Complete** - Core audio done, needs networking + UI

---

## 📞 NEXT STEPS

1. **Continue building backend:**
   - Implement codec.rs
   - Implement transport.rs
   - Wire up AppState

2. **Build React UI:**
   - PTT button component
   - Channel selector
   - Device list
   - Status indicators

3. **Testing:**
   - Same-network communication
   - Audio quality
   - Latency measurements

4. **Polish:**
   - Error handling
   - UI animations
   - Settings panel

---

## 💡 DESKTOP vs ANDROID DIFFERENCES

| Feature | Android | Desktop |
|---------|---------|---------|
| Transport | Bluetooth RFCOMM | UDP Multicast (WiFi) |
| Discovery | Bluetooth pairing | Automatic (multicast) |
| Audio API | AudioRecord/Track | CPAL (cross-platform) |
| UI | egui (Rust) | React + Tauri |
| Range | 10-100m | WiFi range (50-300m) |
| Pairing | Required | Not required |
| Group Calls | No (1-to-1) | Yes (multicast) |

---

**Desktop Version Status:** 20% Complete (audio done, needs networking + UI)  
**Android Version Status:** 100% Complete (ready for production)

**Recommendation:** Finish Android first, launch it, then complete desktop as v2.0 feature.

---

*Document Version: 1.0*  
*Last Updated: January 14, 2025*  
*Status: In Progress*
