// Sassy-Talk - Retro PTT Walkie-Talkie UI
// Copyright 2025 Sassy Consulting LLC

import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import './styles/retro.css';

interface Status {
  connected: boolean;
  transmitting: boolean;
  receiving: boolean;
  channel: number;
  peer_count: number;
  signal_strength: number;
}

interface DeviceInfo {
  device_id: string;
  device_name: string;
  platform: string;
  version: string;
}

interface PeerInfo {
  device_id: number;
  device_name: string;
  channel: number;
  signal_strength: number;
}

function App() {
  const [status, setStatus] = useState<Status>({
    connected: false,
    transmitting: false,
    receiving: false,
    channel: 1,
    peer_count: 0,
    signal_strength: -50,
  });
  const [deviceInfo, setDeviceInfo] = useState<DeviceInfo | null>(null);
  const [peers, setPeers] = useState<PeerInfo[]>([]);
  const [isDiscovering, setIsDiscovering] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [volume, setVolume] = useState({ input: 80, output: 80 });

  // Load initial state
  useEffect(() => {
    const init = async () => {
      try {
        const info = await invoke<DeviceInfo>('get_device_info');
        setDeviceInfo(info);
        
        const vol = await invoke<{ input: number; output: number }>('get_volume');
        setVolume(vol);
      } catch (e) {
        console.error('Init error:', e);
      }
    };
    init();
  }, []);

  // Poll status
  useEffect(() => {
    const interval = setInterval(async () => {
      try {
        const s = await invoke<Status>('get_status');
        setStatus(s);
        
        if (isDiscovering) {
          const p = await invoke<PeerInfo[]>('get_nearby_devices');
          setPeers(p);
        }
      } catch (e) {
        console.error('Status poll error:', e);
      }
    }, 500);
    
    return () => clearInterval(interval);
  }, [isDiscovering]);

  // PTT handlers
  const handlePTTDown = useCallback(async () => {
    try {
      await invoke('start_transmit');
    } catch (e) {
      console.error('PTT start error:', e);
    }
  }, []);

  const handlePTTUp = useCallback(async () => {
    try {
      await invoke('stop_transmit');
    } catch (e) {
      console.error('PTT stop error:', e);
    }
  }, []);

  // Discovery
  const toggleDiscovery = async () => {
    try {
      if (isDiscovering) {
        await invoke('stop_discovery');
        setIsDiscovering(false);
      } else {
        await invoke('start_discovery');
        setIsDiscovering(true);
      }
    } catch (e) {
      console.error('Discovery error:', e);
    }
  };

  // Channel change
  const changeChannel = async (delta: number) => {
    const newChannel = Math.max(1, Math.min(16, status.channel + delta));
    try {
      await invoke('set_channel', { channel: newChannel });
    } catch (e) {
      console.error('Channel change error:', e);
    }
  };

  // Signal bars
  const getSignalBars = (strength: number) => {
    const bars = Math.max(0, Math.min(5, Math.floor((strength + 100) / 20)));
    return '█'.repeat(bars) + '░'.repeat(5 - bars);
  };

  return (
    <div className="walkie-talkie">
      {/* Top LCD Display */}
      <div className="lcd-display">
        <div className="lcd-row">
          <span className="lcd-label">CH</span>
          <span className="lcd-value channel">{status.channel.toString().padStart(2, '0')}</span>
          <span className="lcd-signal">{getSignalBars(status.signal_strength)}</span>
        </div>
        <div className="lcd-row">
          <span className="lcd-status">
            {status.transmitting ? '>>> TX >>>' : 
             status.receiving ? '<<< RX <<<' : 
             status.connected ? 'STANDBY' : 'NO SIGNAL'}
          </span>
        </div>
        <div className="lcd-row small">
          <span>{status.peer_count} PEER{status.peer_count !== 1 ? 'S' : ''}</span>
          <span>{deviceInfo?.device_id || '--------'}</span>
        </div>
      </div>

      {/* Antenna Indicator */}
      <div className={`antenna ${status.connected ? 'active' : ''}`}>
        <div className="antenna-bar"></div>
        <div className="antenna-tip"></div>
      </div>

      {/* Channel Controls */}
      <div className="channel-controls">
        <button className="btn-channel" onClick={() => changeChannel(-1)}>▼</button>
        <div className="channel-display">
          <span className="channel-label">CHANNEL</span>
          <span className="channel-number">{status.channel}</span>
        </div>
        <button className="btn-channel" onClick={() => changeChannel(1)}>▲</button>
      </div>

      {/* PTT Button */}
      <button
        className={`ptt-button ${status.transmitting ? 'active' : ''}`}
        onMouseDown={handlePTTDown}
        onMouseUp={handlePTTUp}
        onMouseLeave={handlePTTUp}
        onTouchStart={handlePTTDown}
        onTouchEnd={handlePTTUp}
      >
        <span className="ptt-label">PUSH TO TALK</span>
        <span className="ptt-icon">{status.transmitting ? '🔴' : '⚫'}</span>
      </button>

      {/* Control Buttons */}
      <div className="control-buttons">
        <button 
          className={`btn-control ${isDiscovering ? 'active' : ''}`}
          onClick={toggleDiscovery}
        >
          {isDiscovering ? 'SCANNING...' : 'SCAN'}
        </button>
        <button 
          className="btn-control"
          onClick={() => setShowSettings(!showSettings)}
        >
          SETTINGS
        </button>
      </div>

      {/* Peer List */}
      {isDiscovering && peers.length > 0 && (
        <div className="peer-list">
          <div className="peer-header">NEARBY UNITS</div>
          {peers.map((peer) => (
            <div 
              key={peer.device_id} 
              className="peer-item"
              onClick={() => invoke('connect_to_peer', { peerId: peer.device_id })}
            >
              <span className="peer-name">{peer.device_name}</span>
              <span className="peer-channel">CH{peer.channel}</span>
            </div>
          ))}
        </div>
      )}

      {/* Settings Panel */}
      {showSettings && (
        <div className="settings-panel">
          <div className="settings-header">SETTINGS</div>
          
          <div className="setting-row">
            <label>MIC VOLUME</label>
            <input 
              type="range" 
              min="0" 
              max="100" 
              value={volume.input}
              onChange={(e) => {
                const v = parseInt(e.target.value);
                setVolume(prev => ({ ...prev, input: v }));
                invoke('set_volume', { input: v });
              }}
            />
            <span>{volume.input}%</span>
          </div>
          
          <div className="setting-row">
            <label>SPEAKER</label>
            <input 
              type="range" 
              min="0" 
              max="100" 
              value={volume.output}
              onChange={(e) => {
                const v = parseInt(e.target.value);
                setVolume(prev => ({ ...prev, output: v }));
                invoke('set_volume', { output: v });
              }}
            />
            <span>{volume.output}%</span>
          </div>
          
          <div className="setting-row">
            <label>ROGER BEEP</label>
            <button 
              className="btn-toggle"
              onClick={() => invoke('set_roger_beep', { enabled: true })}
            >
              ON
            </button>
          </div>
          
          <div className="device-info">
            <div>Device: {deviceInfo?.device_name}</div>
            <div>ID: {deviceInfo?.device_id}</div>
            <div>Platform: {deviceInfo?.platform}</div>
            <div>Version: {deviceInfo?.version}</div>
          </div>
        </div>
      )}

      {/* Footer */}
      <div className="footer">
        <span>SASSY-TALK</span>
        <span className="model">v{deviceInfo?.version || '1.0.0'}</span>
      </div>
    </div>
  );
}

export default App;
