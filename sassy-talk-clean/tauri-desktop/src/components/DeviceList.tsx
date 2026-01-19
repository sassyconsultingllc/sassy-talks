import './DeviceList.css';
import { IconLobby } from './Icons';

interface PeerInfo {
  device_id: number;
  device_name: string;
  address: string;
  last_seen: number;
  channel: number;
}

interface DeviceListProps {
  peers: PeerInfo[];
  currentChannel: number;
}

function DeviceList({ peers, currentChannel }: DeviceListProps) {
  const activePeers = peers.filter(peer => peer.channel === currentChannel);
  const otherPeers = peers.filter(peer => peer.channel !== currentChannel);

  const formatDeviceId = (id: number) => {
    return id.toString(16).toUpperCase().padStart(8, '0');
  };

  const getTimeSince = (timestamp: number) => {
    const now = Date.now() / 1000;
    const diff = now - timestamp;
    
    if (diff < 60) return 'Just now';
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
    return `${Math.floor(diff / 3600)}h ago`;
  };

  return (
    <div className="device-list">
      <div className="device-list-header">
        <h3><IconLobby size={18} /> Nearby Devices</h3>
        <span className="device-count">
          {peers.length} {peers.length === 1 ? 'device' : 'devices'}
        </span>
      </div>

      {peers.length === 0 ? (
        <div className="device-list-empty">
          <p>No devices found</p>
          <p className="hint">Make sure you're on the same WiFi network</p>
        </div>
      ) : (
        <div className="device-list-content">
          {activePeers.length > 0 && (
            <div className="device-section">
              <h4>On Your Channel ({currentChannel})</h4>
              {activePeers.map(peer => (
                <div key={peer.device_id} className="device-item active">
                  <div className="device-info">
                    <div className="device-name">{peer.device_name}</div>
                    <div className="device-details">
                      ID: {formatDeviceId(peer.device_id)} • CH {peer.channel}
                    </div>
                  </div>
                  <div className="device-status">
                    <span className="status-indicator online"></span>
                    <span className="status-text">{getTimeSince(peer.last_seen)}</span>
                  </div>
                </div>
              ))}
            </div>
          )}

          {otherPeers.length > 0 && (
            <div className="device-section">
              <h4>Other Channels</h4>
              {otherPeers.map(peer => (
                <div key={peer.device_id} className="device-item">
                  <div className="device-info">
                    <div className="device-name">{peer.device_name}</div>
                    <div className="device-details">
                      ID: {formatDeviceId(peer.device_id)} • CH {peer.channel}
                    </div>
                  </div>
                  <div className="device-status">
                    <span className="status-indicator"></span>
                    <span className="status-text">{getTimeSince(peer.last_seen)}</span>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export default DeviceList;
