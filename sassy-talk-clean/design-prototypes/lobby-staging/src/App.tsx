// App.tsx - Sassy-Talk Main UI with Lobby System
// Copyright 2025 Sassy Consulting LLC. All rights reserved.

import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './styles/app.css';

// Types matching Rust structs
interface LobbyPeer {
  device_id: number;
  device_name: string;
  platform: string;
  channel: number;
  status: ConnectionStatus;
  signal_strength: number;
  discovered_at: number;
  last_seen: number;
  transport: string | null;
  is_trusted: boolean;
  nickname: string | null;
}

type ConnectionStatus = 
  | 'Discovered'
  | 'RequestSent'
  | 'RequestReceived'
  | 'Connecting'
  | 'Connected'
  | 'Declined'
  | 'Failed'
  | 'Trusted';

interface ConnectionRequest {
  from: LobbyPeer;
  request_id: number;
}

interface AppStatus {
  device_id: number;
  device_name: string;
  channel: number;
  is_transmitting: boolean;
  is_receiving: boolean;
  connected_peers: number;
  transport: string | null;
}

type View = 'lobby' | 'walkie' | 'settings';

export default function App() {
  // View state
  const [currentView, setCurrentView] = useState<View>('lobby');
  
  // Lobby state
  const [peers, setPeers] = useState<LobbyPeer[]>([]);
  const [pendingRequests, setPendingRequests] = useState<ConnectionRequest[]>([]);
  const [isSearching, setIsSearching] = useState(false);
  
  // Walkie state
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [isTransmitting, setIsTransmitting] = useState(false);
  const [channel, setChannel] = useState(1);
  const [connectedPeer, setConnectedPeer] = useState<LobbyPeer | null>(null);
  
  // Settings state
  const [micVolume, setMicVolume] = useState(80);
  const [speakerVolume, setSpeakerVolume] = useState(80);
  const [rogerBeep, setRogerBeep] = useState(true);
  const [autoAcceptTrusted, setAutoAcceptTrusted] = useState(false);
  const [autoAcceptSingle, setAutoAcceptSingle] = useState(false);
  
  // Audio visualization
  const [audioLevel, setAudioLevel] = useState(0);
  
  // Refs
  const pttButtonRef = useRef<HTMLButtonElement>(null);

  // Initialize and setup event listeners
  useEffect(() => {
    const setup = async () => {
      try {
        const initialStatus = await invoke<AppStatus>('get_status');
        setStatus(initialStatus);
        setChannel(initialStatus.channel);
      } catch (e) {
        console.error('Failed to get initial status:', e);
      }
    };
    
    setup();
    
    // Listen for lobby events
    const unlistenPeerDiscovered = listen<LobbyPeer>('peer_discovered', (event) => {
      setPeers(prev => {
        const existing = prev.findIndex(p => p.device_id === event.payload.device_id);
        if (existing >= 0) {
          const updated = [...prev];
          updated[existing] = event.payload;
          return updated;
        }
        return [...prev, event.payload];
      });
    });
    
    const unlistenPeerLost = listen<number>('peer_lost', (event) => {
      setPeers(prev => prev.filter(p => p.device_id !== event.payload));
    });
    
    const unlistenPeerUpdated = listen<LobbyPeer>('peer_updated', (event) => {
      setPeers(prev => prev.map(p => 
        p.device_id === event.payload.device_id ? event.payload : p
      ));
    });
    
    const unlistenConnectionRequest = listen<ConnectionRequest>('connection_request', (event) => {
      setPendingRequests(prev => [...prev, event.payload]);
    });
    
    const unlistenConnected = listen<{ peer_id: number; transport: string }>('connected', (event) => {
      const peer = peers.find(p => p.device_id === event.payload.peer_id);
      if (peer) {
        setConnectedPeer(peer);
        setCurrentView('walkie');
      }
    });
    
    const unlistenAudioLevel = listen<number>('audio_level', (event) => {
      setAudioLevel(event.payload);
    });
    
    // Status polling
    const statusInterval = setInterval(async () => {
      try {
        const s = await invoke<AppStatus>('get_status');
        setStatus(s);
      } catch (e) {
        // Ignore polling errors
      }
    }, 500);
    
    return () => {
      unlistenPeerDiscovered.then(f => f());
      unlistenPeerLost.then(f => f());
      unlistenPeerUpdated.then(f => f());
      unlistenConnectionRequest.then(f => f());
      unlistenConnected.then(f => f());
      unlistenAudioLevel.then(f => f());
      clearInterval(statusInterval);
    };
  }, [peers]);

  // Lobby functions
  const enterLobby = async () => {
    setIsSearching(true);
    try {
      await invoke('enter_lobby');
      await invoke('start_discovery');
    } catch (e) {
      console.error('Failed to enter lobby:', e);
      setIsSearching(false);
    }
  };

  const leaveLobby = async () => {
    setIsSearching(false);
    try {
      await invoke('leave_lobby');
      await invoke('stop_discovery');
    } catch (e) {
      console.error('Failed to leave lobby:', e);
    }
    setPeers([]);
  };

  const requestConnection = async (peerId: number) => {
    try {
      await invoke('request_connection', { peerId });
    } catch (e) {
      console.error('Failed to request connection:', e);
    }
  };

  const acceptConnection = async (requestId: number) => {
    try {
      await invoke('accept_connection', { requestId });
      setPendingRequests(prev => prev.filter(r => r.request_id !== requestId));
    } catch (e) {
      console.error('Failed to accept connection:', e);
    }
  };

  const declineConnection = async (requestId: number) => {
    try {
      await invoke('decline_connection', { requestId });
      setPendingRequests(prev => prev.filter(r => r.request_id !== requestId));
    } catch (e) {
      console.error('Failed to decline connection:', e);
    }
  };

  const trustDevice = async (peerId: number) => {
    try {
      await invoke('trust_device', { peerId });
    } catch (e) {
      console.error('Failed to trust device:', e);
    }
  };

  // Walkie functions
  const startTransmit = async () => {
    try {
      await invoke('start_transmit');
      setIsTransmitting(true);
    } catch (e) {
      console.error('Failed to start transmit:', e);
    }
  };

  const stopTransmit = async () => {
    try {
      await invoke('stop_transmit');
      setIsTransmitting(false);
    } catch (e) {
      console.error('Failed to stop transmit:', e);
    }
  };

  const changeChannel = async (delta: number) => {
    const newChannel = Math.max(1, Math.min(16, channel + delta));
    setChannel(newChannel);
    try {
      await invoke('set_channel', { channel: newChannel });
    } catch (e) {
      console.error('Failed to change channel:', e);
    }
  };

  const disconnect = async () => {
    try {
      await invoke('disconnect');
      setConnectedPeer(null);
      setCurrentView('lobby');
    } catch (e) {
      console.error('Failed to disconnect:', e);
    }
  };

  // PTT handlers
  const handlePttDown = useCallback(() => {
    if (!isTransmitting) startTransmit();
  }, [isTransmitting]);

  const handlePttUp = useCallback(() => {
    if (isTransmitting) stopTransmit();
  }, [isTransmitting]);

  // Render helpers
  const getStatusIcon = (peerStatus: ConnectionStatus) => {
    switch (peerStatus) {
      case 'Connected': return '🟢';
      case 'Connecting': return '🟡';
      case 'RequestSent': return '⏳';
      case 'RequestReceived': return '📨';
      case 'Trusted': return '⭐';
      case 'Declined': return '🔴';
      case 'Failed': return '❌';
      default: return '⚪';
    }
  };

  const getPlatformIcon = (platform: string) => {
    switch (platform) {
      case 'Android': return '🤖';
      case 'IOS': return '🍎';
      case 'MacOS': return '💻';
      case 'Windows': return '🪟';
      case 'Linux': return '🐧';
      default: return '📱';
    }
  };

  const getSignalBars = (strength: number) => {
    const bars = Math.ceil(strength / 20);
    return '▁▃▅▇█'.slice(0, bars);
  };

  // Render Lobby View
  const renderLobby = () => (
    <div className="view lobby-view">
      <header className="lobby-header">
        <h1>Sassy-Talk</h1>
        <p className="subtitle">Waiting Room</p>
      </header>

      {/* Search Toggle */}
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

      {/* Pending Requests */}
      {pendingRequests.length > 0 && (
        <div className="requests-section">
          <h3>Incoming Requests</h3>
          {pendingRequests.map((req) => (
            <div key={req.request_id} className="request-card">
              <div className="request-info">
                <span className="platform-icon">{getPlatformIcon(req.from.platform)}</span>
                <span className="peer-name">{req.from.device_name}</span>
              </div>
              <div className="request-actions">
                <button 
                  className="accept-btn"
                  onClick={() => acceptConnection(req.request_id)}
                >
                  ✓ Accept
                </button>
                <button 
                  className="decline-btn"
                  onClick={() => declineConnection(req.request_id)}
                >
                  ✗ Decline
                </button>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Peer List */}
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
            <div 
              key={peer.device_id} 
              className={`peer-card ${peer.status.toLowerCase()}`}
            >
              <div className="peer-main">
                <span className="status-icon">{getStatusIcon(peer.status)}</span>
                <span className="platform-icon">{getPlatformIcon(peer.platform)}</span>
                <div className="peer-details">
                  <span className="peer-name">
                    {peer.nickname || peer.device_name}
                    {peer.is_trusted && <span className="trusted-badge">★</span>}
                  </span>
                  <span className="peer-meta">
                    CH{peer.channel.toString().padStart(2, '0')} • 
                    {peer.transport || 'Unknown'} • 
                    {getSignalBars(peer.signal_strength)}
                  </span>
                </div>
              </div>
              
              <div className="peer-actions">
                {peer.status === 'Discovered' && (
                  <button 
                    className="connect-btn"
                    onClick={() => requestConnection(peer.device_id)}
                  >
                    Connect
                  </button>
                )}
                {peer.status === 'RequestSent' && (
                  <span className="status-text">Waiting...</span>
                )}
                {peer.status === 'Connected' && (
                  <button 
                    className="connected-btn"
                    onClick={() => {
                      setConnectedPeer(peer);
                      setCurrentView('walkie');
                    }}
                  >
                    Open
                  </button>
                )}
                {!peer.is_trusted && peer.status === 'Connected' && (
                  <button 
                    className="trust-btn"
                    onClick={() => trustDevice(peer.device_id)}
                    title="Trust this device"
                  >
                    ⭐
                  </button>
                )}
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* Bottom Navigation */}
      <nav className="bottom-nav">
        <button className="nav-btn active" onClick={() => setCurrentView('lobby')}>
          <span className="nav-icon">📡</span>
          <span>Lobby</span>
        </button>
        <button 
          className="nav-btn" 
          onClick={() => setCurrentView('walkie')}
          disabled={!connectedPeer}
        >
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

  // Render Walkie View
  const renderWalkie = () => (
    <div className="view walkie-view">
      {/* Connected Peer Info */}
      <header className="walkie-header">
        <div className="connection-info">
          {connectedPeer ? (
            <>
              <span className="connected-to">Connected to</span>
              <span className="peer-name">{connectedPeer.nickname || connectedPeer.device_name}</span>
              <span className="transport-badge">{connectedPeer.transport}</span>
            </>
          ) : (
            <span className="not-connected">Not Connected</span>
          )}
        </div>
        <button className="disconnect-btn" onClick={disconnect}>
          ✕
        </button>
      </header>

      {/* Channel Display */}
      <div className="channel-display">
        <button className="channel-btn" onClick={() => changeChannel(-1)}>◀</button>
        <div className="channel-lcd">
          <span className="channel-label">CH</span>
          <span className="channel-number">{channel.toString().padStart(2, '0')}</span>
        </div>
        <button className="channel-btn" onClick={() => changeChannel(1)}>▶</button>
      </div>

      {/* Status Display */}
      <div className="status-display">
        <div className={`status-indicator ${isTransmitting ? 'tx' : 'rx'}`}>
          {isTransmitting ? 'TRANSMITTING' : 'STANDBY'}
        </div>
        <div className="audio-meter">
          <div 
            className="audio-level" 
            style={{ width: `${audioLevel}%` }}
          ></div>
        </div>
      </div>

      {/* PTT Button */}
      <div className="ptt-container">
        <button
          ref={pttButtonRef}
          className={`ptt-btn ${isTransmitting ? 'active' : ''}`}
          onMouseDown={handlePttDown}
          onMouseUp={handlePttUp}
          onMouseLeave={handlePttUp}
          onTouchStart={(e) => { e.preventDefault(); handlePttDown(); }}
          onTouchEnd={(e) => { e.preventDefault(); handlePttUp(); }}
          disabled={!connectedPeer}
        >
          <span className="ptt-icon">🎙️</span>
          <span className="ptt-text">
            {isTransmitting ? 'RELEASE TO STOP' : 'PUSH TO TALK'}
          </span>
        </button>
      </div>

      {/* Quick Settings */}
      <div className="quick-settings">
        <div className="volume-control">
          <span className="volume-icon">🔊</span>
          <input 
            type="range" 
            min="0" 
            max="100" 
            value={speakerVolume}
            onChange={(e) => setSpeakerVolume(Number(e.target.value))}
          />
        </div>
        <div className="volume-control">
          <span className="volume-icon">🎤</span>
          <input 
            type="range" 
            min="0" 
            max="100" 
            value={micVolume}
            onChange={(e) => setMicVolume(Number(e.target.value))}
          />
        </div>
      </div>

      {/* Bottom Navigation */}
      <nav className="bottom-nav">
        <button className="nav-btn" onClick={() => setCurrentView('lobby')}>
          <span className="nav-icon">📡</span>
          <span>Lobby</span>
        </button>
        <button className="nav-btn active" onClick={() => setCurrentView('walkie')}>
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

  // Render Settings View
  const renderSettings = () => (
    <div className="view settings-view">
      <header className="settings-header">
        <h1>Settings</h1>
      </header>

      <div className="settings-content">
        {/* Device Info */}
        <section className="settings-section">
          <h3>Device</h3>
          <div className="setting-row">
            <span className="setting-label">Device ID</span>
            <span className="setting-value">
              {status?.device_id.toString(16).toUpperCase().padStart(8, '0') || '--------'}
            </span>
          </div>
          <div className="setting-row">
            <span className="setting-label">Device Name</span>
            <span className="setting-value">{status?.device_name || 'Unknown'}</span>
          </div>
        </section>

        {/* Audio Settings */}
        <section className="settings-section">
          <h3>Audio</h3>
          <div className="setting-row">
            <span className="setting-label">Microphone Volume</span>
            <input 
              type="range" 
              min="0" 
              max="100" 
              value={micVolume}
              onChange={(e) => setMicVolume(Number(e.target.value))}
            />
            <span className="setting-value">{micVolume}%</span>
          </div>
          <div className="setting-row">
            <span className="setting-label">Speaker Volume</span>
            <input 
              type="range" 
              min="0" 
              max="100" 
              value={speakerVolume}
              onChange={(e) => setSpeakerVolume(Number(e.target.value))}
            />
            <span className="setting-value">{speakerVolume}%</span>
          </div>
          <div className="setting-row">
            <span className="setting-label">Roger Beep</span>
            <label className="toggle">
              <input 
                type="checkbox" 
                checked={rogerBeep}
                onChange={(e) => setRogerBeep(e.target.checked)}
              />
              <span className="toggle-slider"></span>
            </label>
          </div>
        </section>

        {/* Connection Settings */}
        <section className="settings-section">
          <h3>Connections</h3>
          <div className="setting-row">
            <span className="setting-label">Auto-accept trusted devices</span>
            <label className="toggle">
              <input 
                type="checkbox" 
                checked={autoAcceptTrusted}
                onChange={(e) => setAutoAcceptTrusted(e.target.checked)}
              />
              <span className="toggle-slider"></span>
            </label>
          </div>
          <div className="setting-row">
            <span className="setting-label">Auto-accept when only one peer</span>
            <label className="toggle">
              <input 
                type="checkbox" 
                checked={autoAcceptSingle}
                onChange={(e) => setAutoAcceptSingle(e.target.checked)}
              />
              <span className="toggle-slider"></span>
            </label>
          </div>
        </section>

        {/* Trusted Devices */}
        <section className="settings-section">
          <h3>Trusted Devices</h3>
          <div className="trusted-list">
            {peers.filter(p => p.is_trusted).map(peer => (
              <div key={peer.device_id} className="trusted-device">
                <span>{peer.nickname || peer.device_name}</span>
                <button 
                  className="untrust-btn"
                  onClick={() => invoke('untrust_device', { peerId: peer.device_id })}
                >
                  Remove
                </button>
              </div>
            ))}
            {peers.filter(p => p.is_trusted).length === 0 && (
              <p className="no-trusted">No trusted devices yet</p>
            )}
          </div>
        </section>

        {/* About */}
        <section className="settings-section">
          <h3>About</h3>
          <div className="setting-row">
            <span className="setting-label">Version</span>
            <span className="setting-value">1.0.0</span>
          </div>
          <div className="setting-row">
            <span className="setting-label">© 2025 Sassy Consulting LLC</span>
          </div>
        </section>
      </div>

      {/* Bottom Navigation */}
      <nav className="bottom-nav">
        <button className="nav-btn" onClick={() => setCurrentView('lobby')}>
          <span className="nav-icon">📡</span>
          <span>Lobby</span>
        </button>
        <button 
          className="nav-btn" 
          onClick={() => setCurrentView('walkie')}
          disabled={!connectedPeer}
        >
          <span className="nav-icon">📻</span>
          <span>Talk</span>
        </button>
        <button className="nav-btn active" onClick={() => setCurrentView('settings')}>
          <span className="nav-icon">⚙️</span>
          <span>Settings</span>
        </button>
      </nav>
    </div>
  );

  return (
    <div className="app">
      {currentView === 'lobby' && renderLobby()}
      {currentView === 'walkie' && renderWalkie()}
      {currentView === 'settings' && renderSettings()}
    </div>
  );
}
