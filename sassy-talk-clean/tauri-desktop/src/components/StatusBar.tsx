import './StatusBar.css';

interface AppStatus {
  connection_status: string;
  channel: number;
  peer_count: number;
  is_transmitting: boolean;
}

interface StatusBarProps {
  status: AppStatus | null;
  isConnected: boolean;
  peerCount: number;
}

function StatusBar({ status, isConnected, peerCount }: StatusBarProps) {
  const getStatusText = () => {
    if (!status) return 'Initializing...';
    
    if (status.is_transmitting) {
      return `TRANSMITTING ON CH ${status.channel}`;
    }
    
    if (status.connection_status === 'Receiving') {
      return 'RECEIVING AUDIO';
    }
    
    if (isConnected) {
      return peerCount > 0 
        ? `Connected • ${peerCount} ${peerCount === 1 ? 'peer' : 'peers'}`
        : 'Listening for devices...';
    }
    
    return 'Disconnected';
  };

  const getStatusClass = () => {
    if (!status) return '';
    
    if (status.is_transmitting) return 'transmitting';
    if (status.connection_status === 'Receiving') return 'receiving';
    if (isConnected) return 'connected';
    return 'disconnected';
  };

  return (
    <div className={`status-bar ${getStatusClass()}`}>
      <div className="status-content">
        <div className="status-indicator">
          <span className="status-dot"></span>
        </div>
        <div className="status-text">
          {getStatusText()}
        </div>
      </div>
      <div className="status-footer">
        <span className="version">v1.0.0</span>
        <span className="encryption">🔒 AES-256</span>
      </div>
    </div>
  );
}

export default StatusBar;
