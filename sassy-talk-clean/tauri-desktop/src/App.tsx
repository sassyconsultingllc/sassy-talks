import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import './styles/App.css';
import PTTButton from './components/PTTButton';
import ChannelSelector from './components/ChannelSelector';
import DeviceList from './components/DeviceList';
import StatusBar from './components/StatusBar';
import SettingsPanel from './components/SettingsPanel';

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

function App() {
  const [channel, setChannel] = useState(1);
  const [isTransmitting, setIsTransmitting] = useState(false);
  const [isConnected, setIsConnected] = useState(false);
  const [peers, setPeers] = useState<PeerInfo[]>([]);
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Initialize app
  useEffect(() => {
    initializeApp();
    
    // Update status every second
    const statusInterval = setInterval(updateStatus, 1000);
    
    return () => {
      clearInterval(statusInterval);
      cleanup();
    };
  }, []);

  const initializeApp = async () => {
    try {
      await invoke('start_discovery');
      setIsConnected(true);
    } catch (err) {
      setError(`Failed to start: ${err}`);
      console.error('Initialization error:', err);
    }
  };

  const cleanup = async () => {
    try {
      await invoke('stop_discovery');
    } catch (err) {
      console.error('Cleanup error:', err);
    }
  };

  const updateStatus = async () => {
    try {
      const newStatus = await invoke<AppStatus>('get_status');
      setStatus(newStatus);
      
      const nearbyDevices = await invoke<PeerInfo[]>('get_nearby_devices');
      setPeers(nearbyDevices);
      
      setIsConnected(newStatus.connection_status !== 'Disconnected');
      setChannel(newStatus.channel);
    } catch (err) {
      console.error('Status update error:', err);
    }
  };

  const handlePTTPress = async () => {
    try {
      await invoke('start_transmit');
      setIsTransmitting(true);
    } catch (err) {
      setError(`Failed to transmit: ${err}`);
      console.error('Transmit error:', err);
    }
  };

  const handlePTTRelease = async () => {
    try {
      await invoke('stop_transmit');
      setIsTransmitting(false);
    } catch (err) {
      setError(`Failed to stop transmit: ${err}`);
      console.error('Stop transmit error:', err);
    }
  };

  const handleChannelChange = async (newChannel: number) => {
    try {
      await invoke('set_channel', { channel: newChannel });
      setChannel(newChannel);
    } catch (err) {
      setError(`Failed to change channel: ${err}`);
      console.error('Channel change error:', err);
    }
  };

  return (
    <div className="app">
      {/* Header */}
      <header className="app-header">
        <div className="header-content">
          <h1>🎙️ Sassy-Talk</h1>
          <button 
            className="settings-button"
            onClick={() => setShowSettings(!showSettings)}
            title="Settings"
          >
            ⚙️
          </button>
        </div>
      </header>

      {/* Error Display */}
      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button onClick={() => setError(null)}>✕</button>
        </div>
      )}

      {/* Main Content */}
      <main className="app-main">
        {/* Channel Selector */}
        <ChannelSelector 
          channel={channel} 
          onChange={handleChannelChange}
        />

        {/* PTT Button */}
        <PTTButton
          isTransmitting={isTransmitting}
          isConnected={isConnected}
          onPress={handlePTTPress}
          onRelease={handlePTTRelease}
        />

        {/* Device List */}
        <DeviceList peers={peers} currentChannel={channel} />

        {/* Status Bar */}
        <StatusBar 
          status={status}
          isConnected={isConnected}
          peerCount={peers.length}
        />
      </main>

      {/* Settings Panel */}
      {showSettings && (
        <SettingsPanel onClose={() => setShowSettings(false)} />
      )}
    </div>
  );
}

export default App;
