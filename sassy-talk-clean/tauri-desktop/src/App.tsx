// App.tsx - Sassy-Talk Main UI with Lobby System
// Copyright 2025 Sassy Consulting LLC. All rights reserved.
// Production Build - Full Implementation

import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import './styles/app.css';
import Sounds from './sounds';

// ============================================================================
// Types matching Rust backend
// ============================================================================

interface PeerInfo {
  device_id: number;
  device_name: string;
  address: string;
  last_seen: number;
  channel: number;
}

interface AppStatus {
  connection_status: string;
  channel: number;
  peer_count: number;
  is_transmitting: boolean;
}

interface DeviceInfo {
  device_id: string;
  device_name: string;
  version: string;
}

interface Volume {
  input: number;
  output: number;
}

interface AudioDevices {
  inputs: AudioDeviceInfo[];
  outputs: AudioDeviceInfo[];
}

interface AudioDeviceInfo {
  name: string;
  is_default: boolean;
  device_type: string;
}

type View = 'lobby' | 'walkie' | 'settings';

// ============================================================================
// Main App Component
// ============================================================================

export default function App() {
  // View state
  const [currentView, setCurrentView] = useState<View>('lobby');
  
  // Lobby state
  const [peers, setPeers] = useState<PeerInfo[]>([]);
  const [isSearching, setIsSearching] = useState(false);
  
  // Walkie state
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [isTransmitting, setIsTransmitting] = useState(false);
  const [isReceiving, setIsReceiving] = useState(false);
  const [channel, setChannel] = useState(1);
  const [deviceInfo, setDeviceInfo] = useState<DeviceInfo | null>(null);
  
  // Settings state
  const [micVolume, setMicVolume] = useState(80);
  const [speakerVolume, setSpeakerVolume] = useState(80);
  const [rogerBeep, setRogerBeep] = useState(true);
  const [voxEnabled, setVoxEnabled] = useState(false);
  const [voxThreshold, setVoxThreshold] = useState(10);
  const [audioDevices, setAudioDevices] = useState<AudioDevices>({ inputs: [], outputs: [] });
  const [selectedInput, setSelectedInput] = useState<string>('');
  const [selectedOutput, setSelectedOutput] = useState<string>('');
  
  // Audio visualization
  const [audioLevel, setAudioLevel] = useState(0);
  
  // Error handling
  const [error, setError] = useState<string | null>(null);
  
  // Refs
  const pttButtonRef = useRef<HTMLButtonElement>(null);
  const statusIntervalRef = useRef<number | null>(null);

  // ============================================================================
  // Initialization
  // ============================================================================

  useEffect(() => {
    const setup = async () => {
      try {
        // Get device info
        const info = await invoke<DeviceInfo>('get_device_info');
        setDeviceInfo(info);
        
        // Get initial status
        const initialStatus = await invoke<AppStatus>('get_status');
        setStatus(initialStatus);
        setChannel(initialStatus.channel);
        
        // Get volume
        const vol = await invoke<Volume>('get_volume');
        setMicVolume(Math.round(vol.input * 100));
        setSpeakerVolume(Math.round(vol.output * 100));
        
        // Get audio devices
        const devices = await invoke<AudioDevices>('get_audio_devices');
        setAudioDevices(devices);
        
        // Set defaults
        const defaultInput = devices.inputs.find(d => d.is_default);
        const defaultOutput = devices.outputs.find(d => d.is_default);
        if (defaultInput) setSelectedInput(defaultInput.name);
        if (defaultOutput) setSelectedOutput(defaultOutput.name);
        
      } catch (e) {
        console.error('Failed to initialize:', e);
        setError(`Initialization failed: ${e}`);
      }
    };
    
    setup();
    
    // Listen for audio level events
    let unlistenAudio: UnlistenFn | null = null;
    let unlistenReceiving: UnlistenFn | null = null;
    let hadPeers = false;
    let wasReceiving = false;
    
    listen<number>('audio_level', (event) => {
      setAudioLevel(event.payload);
    }).then(fn => { unlistenAudio = fn; });
    
    listen<boolean>('receiving', (event) => {
      setIsReceiving(event.payload);
      // Play incoming transmission sound when we start receiving
      if (event.payload && !wasReceiving) {
        Sounds.incomingTransmission();
      }
      wasReceiving = event.payload;
    }).then(fn => { unlistenReceiving = fn; });
    
    // Status polling
    statusIntervalRef.current = window.setInterval(async () => {
      try {
        const s = await invoke<AppStatus>('get_status');
        setStatus(s);
        setIsTransmitting(s.is_transmitting);
        
        if (isSearching) {
          const nearbyPeers = await invoke<PeerInfo[]>('get_nearby_devices');
          // Play connection success when first peer discovered
          if (nearbyPeers.length > 0 && !hadPeers) {
            Sounds.connectionSuccess();
            hadPeers = true;
          } else if (nearbyPeers.length === 0) {
            hadPeers = false;
          }
          setPeers(nearbyPeers);
        }
      } catch (e) {
        // Ignore polling errors
      }
    }, 250);
    
    return () => {
      if (unlistenAudio) unlistenAudio();
      if (unlistenReceiving) unlistenReceiving();
      if (statusIntervalRef.current) clearInterval(statusIntervalRef.current);
    };
  }, [isSearching]);

  // ==========================================================================
  // Sound Effects (Web Audio API - frontend)
  // ==========================================================================

  const playErrorTone = () => Sounds.error();
  const playDeliveredTone = () => Sounds.messageDelivered();
  const playConnectionTone = () => Sounds.connectionSuccess();
  const playChannelTone = () => Sounds.channelChange();

  // ==========================================================================
  // Lobby Functions
  // ==========================================================================

  const enterLobby = async () => {
    setError(null);
    setIsSearching(true);
    Sounds.discoveryStart();
    try {
      await invoke('start_discovery');
    } catch (e) {
      console.error('Failed to start discovery:', e);
      setError(`Discovery failed: ${e}`);
      setIsSearching(false);
      Sounds.error();
    }
  };

  const leaveLobby = async () => {
    setIsSearching(false);
    Sounds.discoveryStop();
    try {
      await invoke('stop_discovery');
    } catch (e) {
      console.error('Failed to stop discovery:', e);
    }
    setPeers([]);
  };

  const joinPeerChannel = async (peerChannel: number) => {
    try {
      await invoke('set_channel', { channel: peerChannel });
      setChannel(peerChannel);
      Sounds.connectionSuccess(); // Three-tone success chime
      setCurrentView('walkie');
    } catch (e) {
      setError(`Failed to join channel: ${e}`);
      Sounds.error();
    }
  };

  // ============================================================================
  // Walkie Functions
  // ============================================================================

  const startTransmit = async () => {
    if (isTransmitting) return;
    setError(null);
    try {
      await invoke('start_transmit');
      setIsTransmitting(true);
      Sounds.pttStart();
    } catch (e) {
      console.error('Failed to start transmit:', e);
      setError(`Transmit failed: ${e}`);
      Sounds.error();
    }
  };

  const stopTransmit = async () => {
    if (!isTransmitting) return;
    try {
      await invoke('stop_transmit');
      setIsTransmitting(false);
      if (rogerBeep) {
        Sounds.rogerBeep();
      }
    } catch (e) {
      console.error('Failed to stop transmit:', e);
    }
  };

  const changeChannel = async (delta: number) => {
    const newChannel = Math.max(1, Math.min(16, channel + delta));
    if (newChannel === channel) return;
    
    setChannel(newChannel);
    try {
      await invoke('set_channel', { channel: newChannel });
      Sounds.channelChange();
    } catch (e) {
      console.error('Failed to change channel:', e);
      setError(`Channel change failed: ${e}`);
      Sounds.error();
    }
  };

  const disconnect = async () => {
    try {
      await invoke('disconnect');
      setIsSearching(false);
      setPeers([]);
      setCurrentView('lobby');
    } catch (e) {
      console.error('Failed to disconnect:', e);
    }
  };

  // ============================================================================
  // Settings Functions
  // ============================================================================

  const updateVolume = async (input: number, output: number) => {
    try {
      await invoke('set_volume', { input: input / 100, output: output / 100 });
    } catch (e) {
      console.error('Failed to set volume:', e);
    }
  };

  const handleMicVolumeChange = (value: number) => {
    setMicVolume(value);
    updateVolume(value, speakerVolume);
  };

  const handleSpeakerVolumeChange = (value: number) => {
    setSpeakerVolume(value);
    updateVolume(micVolume, value);
  };

  const handleRogerBeepChange = async (enabled: boolean) => {
    setRogerBeep(enabled);
    try {
      await invoke('set_roger_beep', { enabled });
    } catch (e) {
      console.error('Failed to set roger beep:', e);
    }
  };

  const handleVoxChange = async (enabled: boolean) => {
    setVoxEnabled(enabled);
    try {
      await invoke('set_vox_enabled', { enabled });
    } catch (e) {
      console.error('Failed to set VOX:', e);
    }
  };

  const handleVoxThresholdChange = async (value: number) => {
    setVoxThreshold(value);
    try {
      await invoke('set_vox_threshold', { threshold: value / 100 });
    } catch (e) {
      console.error('Failed to set VOX threshold:', e);
    }
  };

  const handleInputDeviceChange = async (deviceName: string) => {
    setSelectedInput(deviceName);
    try {
      await invoke('set_input_device', { device_name: deviceName });
    } catch (e) {
      setError(`Failed to set microphone: ${e}`);
    }
  };

  const handleOutputDeviceChange = async (deviceName: string) => {
    setSelectedOutput(deviceName);
    try {
      await invoke('set_output_device', { device_name: deviceName });
    } catch (e) {
      setError(`Failed to set speaker: ${e}`);
    }
  };

  const refreshAudioDevices = async () => {
    try {
      const devices = await invoke<AudioDevices>('get_audio_devices');
      setAudioDevices(devices);
    } catch (e) {
      console.error('Failed to refresh audio devices:', e);
    }
  };

  // ============================================================================
  // PTT Handlers
  // ============================================================================

  const handlePttDown = useCallback(() => {
    if (!isTransmitting && isSearching) {
      startTransmit();
    }
  }, [isTransmitting, isSearching]);

  const handlePttUp = useCallback(() => {
    if (isTransmitting) {
      stopTransmit();
    }
  }, [isTransmitting]);

  // Keyboard PTT
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.code === 'Space' && !e.repeat && currentView === 'walkie') {
        e.preventDefault();
        handlePttDown();
      }
    };
    
    const handleKeyUp = (e: KeyboardEvent) => {
      if (e.code === 'Space' && currentView === 'walkie') {
        e.preventDefault();
        handlePttUp();
      }
    };
    
    window.addEventListener('keydown', handleKeyDown);
    window.addEventListener('keyup', handleKeyUp);
    
    return () => {
      window.removeEventListener('keydown', handleKeyDown);
      window.removeEventListener('keyup', handleKeyUp);
    };
  }, [currentView, handlePttDown, handlePttUp]);

  // ============================================================================
  // Helpers
  // ============================================================================

  const getPlatformIcon = (deviceName: string): string => {
    const name = deviceName.toLowerCase();
    if (name.includes('android') || name.includes('pixel') || name.includes('galaxy')) return '🤖';
    if (name.includes('iphone') || name.includes('ipad') || name.includes('mac')) return '🍎';
    if (name.includes('windows') || name.includes('surface')) return '🪟';
    if (name.includes('linux') || name.includes('ubuntu')) return '🐧';
    return '📱';
  };

  const getSignalBars = (lastSeen: number): string => {
    const now = Date.now();
    const age = now - lastSeen;
    if (age < 1000) return '▁▃▅▇█';
    if (age < 3000) return '▁▃▅▇';
    if (age < 5000) return '▁▃▅';
    if (age < 10000) return '▁▃';
    return '▁';
  };

  const getStatusText = (): string => {
    if (!status) return 'Initializing...';
    if (isTransmitting) return 'TRANSMITTING';
    if (isReceiving) return 'Receiving...';
    if (isSearching) return `Online • ${peers.length} nearby`;
    return 'Offline';
  };

  // ============================================================================
  // Render: Lobby View
  // ============================================================================

  const renderLobby = () => (
    <div className="view lobby-view">
      <header className="lobby-header">
        <h1>Sassy-Talk</h1>
        <p className="subtitle">Bluetooth Walkie-Talkie</p>
      </header>

      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button onClick={() => setError(null)}>✕</button>
        </div>
      )}

      <div className="search-section">
        <button 
          className={`search-btn ${isSearching ? 'active' : ''}`}
          onClick={isSearching ? leaveLobby : enterLobby}
        >
          <span className="search-icon">{isSearching ? '🔍' : '📡'}</span>
          <span>{isSearching ? 'Searching...' : 'Find Devices'}</span>
        </button>
        {isSearching && (
          <div className="search-pulse">
            <div className="pulse-ring"></div>
            <div className="pulse-ring delay-1"></div>
            <div className="pulse-ring delay-2"></div>
          </div>
        )}
      </div>

      <div className="current-channel-display">
        <span className="channel-label">Your Channel:</span>
        <span className="channel-value">CH{channel.toString().padStart(2, '0')}</span>
      </div>

      <div className="peers-section">
        <h3>Nearby Devices {peers.length > 0 && `(${peers.length})`}</h3>
        
        {peers.length === 0 && isSearching && (
          <div className="no-peers">
            <p>Looking for nearby devices...</p>
            <p className="hint">Make sure other devices have the app open</p>
          </div>
        )}
        
        {peers.length === 0 && !isSearching && (
          <div className="no-peers">
            <p>No devices found</p>
            <p className="hint">Tap "Find Devices" to start searching</p>
          </div>
        )}

        <div className="peer-list">
          {peers.map((peer) => (
            <div key={peer.device_id} className={`peer-card ${peer.channel === channel ? 'same-channel' : ''}`}>
              <div className="peer-main">
                <span className="status-icon">🟢</span>
                <span className="platform-icon">{getPlatformIcon(peer.device_name)}</span>
                <div className="peer-details">
                  <span className="peer-name">{peer.device_name}</span>
                  <span className="peer-meta">
                    CH{peer.channel.toString().padStart(2, '0')} • {getSignalBars(peer.last_seen)}
                  </span>
                </div>
              </div>
              
              <div className="peer-actions">
                {peer.channel === channel ? (
                  <button className="connected-btn" onClick={() => setCurrentView('walkie')}>Talk</button>
                ) : (
                  <button className="connect-btn" onClick={() => joinPeerChannel(peer.channel)}>
                    Join CH{peer.channel.toString().padStart(2, '0')}
                  </button>
                )}
              </div>
            </div>
          ))}
        </div>
      </div>

      {isSearching && (
        <div className="quick-talk-section">
          <button className="quick-talk-btn" onClick={() => setCurrentView('walkie')}>
            <span>📻</span>
            <span>Start Talking on CH{channel.toString().padStart(2, '0')}</span>
          </button>
        </div>
      )}

      <nav className="bottom-nav">
        <button className="nav-btn active">
          <span className="nav-icon">📡</span>
          <span>Lobby</span>
        </button>
        <button className="nav-btn" onClick={() => setCurrentView('walkie')}>
          <span className="nav-icon">📻</span>
          <span>Talk</span>
        </button>
        <button className="nav-btn" onClick={() => setCurrentView('settings')}>
          <span className="nav-icon">⚙️</span>
          <span>Settings</span>
        </button>
      </nav>
    </div>
  );

  // ============================================================================
  // Render: Walkie View
  // ============================================================================

  const renderWalkie = () => (
    <div className="view walkie-view">
      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button onClick={() => setError(null)}>✕</button>
        </div>
      )}

      <header className="walkie-header">
        <div className="connection-info">
          <span className="connected-to">{getStatusText()}</span>
          <span className="peer-name">Channel {channel.toString().padStart(2, '0')}</span>
          {peers.length > 0 && <span className="transport-badge">UDP Multicast</span>}
        </div>
        <button className="disconnect-btn" onClick={disconnect} title="Disconnect">✕</button>
      </header>

      <div className="channel-display">
        <button className="channel-btn" onClick={() => changeChannel(-1)} disabled={channel <= 1}>◀</button>
        <div className="channel-lcd">
          <span className="channel-label">CH</span>
          <span className="channel-number">{channel.toString().padStart(2, '0')}</span>
        </div>
        <button className="channel-btn" onClick={() => changeChannel(1)} disabled={channel >= 16}>▶</button>
      </div>

      <div className="status-display">
        <div className={`status-indicator ${isTransmitting ? 'tx' : isReceiving ? 'rx-active' : 'rx'}`}>
          {isTransmitting ? 'TRANSMITTING' : isReceiving ? 'RECEIVING' : 'STANDBY'}
        </div>
        <div className="audio-meter">
          <div className={`audio-level ${isTransmitting ? 'tx' : isReceiving ? 'rx' : ''}`} style={{ width: `${audioLevel}%` }}></div>
        </div>
      </div>

      <div className="ptt-container">
        <button
          ref={pttButtonRef}
          className={`ptt-btn ${isTransmitting ? 'active' : ''} ${!isSearching ? 'disabled' : ''}`}
          onMouseDown={handlePttDown}
          onMouseUp={handlePttUp}
          onMouseLeave={handlePttUp}
          onTouchStart={(e) => { e.preventDefault(); handlePttDown(); }}
          onTouchEnd={(e) => { e.preventDefault(); handlePttUp(); }}
          disabled={!isSearching}
        >
          <span className="ptt-icon">{isTransmitting ? '🔴' : '🎙️'}</span>
          <span className="ptt-text">
            {!isSearching ? 'START DISCOVERY FIRST' : isTransmitting ? 'RELEASE TO STOP' : 'PUSH TO TALK'}
          </span>
          <span className="ptt-hint">or hold SPACEBAR</span>
        </button>
      </div>

      <div className="quick-settings">
        <div className="volume-control">
          <span className="volume-icon">🔊</span>
          <input type="range" min="0" max="100" value={speakerVolume} onChange={(e) => handleSpeakerVolumeChange(Number(e.target.value))} />
          <span className="volume-value">{speakerVolume}%</span>
        </div>
        <div className="volume-control">
          <span className="volume-icon">🎤</span>
          <input type="range" min="0" max="100" value={micVolume} onChange={(e) => handleMicVolumeChange(Number(e.target.value))} />
          <span className="volume-value">{micVolume}%</span>
        </div>
      </div>

      {peers.length > 0 && (
        <div className="nearby-mini">
          <span className="nearby-label">Nearby:</span>
          {peers.slice(0, 3).map(p => (
            <span key={p.device_id} className="nearby-peer">{getPlatformIcon(p.device_name)} {p.device_name.substring(0, 10)}</span>
          ))}
          {peers.length > 3 && <span className="nearby-more">+{peers.length - 3} more</span>}
        </div>
      )}

      <nav className="bottom-nav">
        <button className="nav-btn" onClick={() => setCurrentView('lobby')}>
          <span className="nav-icon">📡</span>
          <span>Lobby</span>
        </button>
        <button className="nav-btn active">
          <span className="nav-icon">📻</span>
          <span>Talk</span>
        </button>
        <button className="nav-btn" onClick={() => setCurrentView('settings')}>
          <span className="nav-icon">⚙️</span>
          <span>Settings</span>
        </button>
      </nav>
    </div>
  );

  // ============================================================================
  // Render: Settings View
  // ============================================================================

  const renderSettings = () => (
    <div className="view settings-view">
      <header className="settings-header">
        <h1>Settings</h1>
      </header>

      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button onClick={() => setError(null)}>✕</button>
        </div>
      )}

      <div className="settings-content">
        <section className="settings-section">
          <h3>Device</h3>
          <div className="setting-row">
            <span className="setting-label">Device ID</span>
            <span className="setting-value">{deviceInfo?.device_id || '--------'}</span>
          </div>
          <div className="setting-row">
            <span className="setting-label">Device Name</span>
            <span className="setting-value">{deviceInfo?.device_name || 'Unknown'}</span>
          </div>
          <div className="setting-row">
            <span className="setting-label">Version</span>
            <span className="setting-value">{deviceInfo?.version || '0.0.0'}</span>
          </div>
        </section>

        <section className="settings-section">
          <h3>Audio Devices <button className="refresh-btn" onClick={refreshAudioDevices} title="Refresh">🔄</button></h3>
          <div className="setting-row">
            <span className="setting-label">Microphone</span>
            <select value={selectedInput} onChange={(e) => handleInputDeviceChange(e.target.value)} className="device-select">
              {audioDevices.inputs.map(d => (
                <option key={d.name} value={d.name}>{d.name} {d.is_default ? '(Default)' : ''}</option>
              ))}
            </select>
          </div>
          <div className="setting-row">
            <span className="setting-label">Speaker</span>
            <select value={selectedOutput} onChange={(e) => handleOutputDeviceChange(e.target.value)} className="device-select">
              {audioDevices.outputs.map(d => (
                <option key={d.name} value={d.name}>{d.name} {d.is_default ? '(Default)' : ''}</option>
              ))}
            </select>
          </div>
        </section>

        <section className="settings-section">
          <h3>Audio Levels</h3>
          <div className="setting-row">
            <span className="setting-label">Mic Volume</span>
            <input type="range" min="0" max="100" value={micVolume} onChange={(e) => handleMicVolumeChange(Number(e.target.value))} />
            <span className="setting-value">{micVolume}%</span>
          </div>
          <div className="setting-row">
            <span className="setting-label">Speaker Volume</span>
            <input type="range" min="0" max="100" value={speakerVolume} onChange={(e) => handleSpeakerVolumeChange(Number(e.target.value))} />
            <span className="setting-value">{speakerVolume}%</span>
          </div>
        </section>

        <section className="settings-section">
          <h3>Voice Features</h3>
          <div className="setting-row">
            <span className="setting-label">Roger Beep</span>
            <label className="toggle">
              <input type="checkbox" checked={rogerBeep} onChange={(e) => handleRogerBeepChange(e.target.checked)} />
              <span className="toggle-slider"></span>
            </label>
          </div>
          <div className="setting-row">
            <span className="setting-label">VOX (Voice Activated)</span>
            <label className="toggle">
              <input type="checkbox" checked={voxEnabled} onChange={(e) => handleVoxChange(e.target.checked)} />
              <span className="toggle-slider"></span>
            </label>
          </div>
          {voxEnabled && (
            <div className="setting-row">
              <span className="setting-label">VOX Threshold</span>
              <input type="range" min="0" max="100" value={voxThreshold} onChange={(e) => handleVoxThresholdChange(Number(e.target.value))} />
              <span className="setting-value">{voxThreshold}%</span>
            </div>
          )}
        </section>

        <section className="settings-section">
          <h3>Network</h3>
          <div className="setting-row">
            <span className="setting-label">Transport</span>
            <span className="setting-value">UDP Multicast</span>
          </div>
          <div className="setting-row">
            <span className="setting-label">Multicast Group</span>
            <span className="setting-value">239.255.42.42:5555</span>
          </div>
          <div className="setting-row">
            <span className="setting-label">Codec</span>
            <span className="setting-value">Opus 32kbps VBR</span>
          </div>
          <div className="setting-row">
            <span className="setting-label">Frame Size</span>
            <span className="setting-value">20ms (960 samples)</span>
          </div>
        </section>

        <section className="settings-section">
          <h3>About</h3>
          <div className="setting-row">
            <span className="setting-label">© 2025 Sassy Consulting LLC</span>
          </div>
        </section>
      </div>

      <nav className="bottom-nav">
        <button className="nav-btn" onClick={() => setCurrentView('lobby')}>
          <span className="nav-icon">📡</span>
          <span>Lobby</span>
        </button>
        <button className="nav-btn" onClick={() => setCurrentView('walkie')}>
          <span className="nav-icon">📻</span>
          <span>Talk</span>
        </button>
        <button className="nav-btn active">
          <span className="nav-icon">⚙️</span>
          <span>Settings</span>
        </button>
      </nav>
    </div>
  );

  // ============================================================================
  // Main Render
  // ============================================================================

  return (
    <div className="app">
      {currentView === 'lobby' && renderLobby()}
      {currentView === 'walkie' && renderWalkie()}
      {currentView === 'settings' && renderSettings()}
    </div>
  );
}
