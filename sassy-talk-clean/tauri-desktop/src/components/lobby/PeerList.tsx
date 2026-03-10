import UserAvatar from '../../components/UserAvatar';
import { IconSignal, getSignalLevel } from '../../components/Icons';
import type { PeerInfo } from '../../types';

export default function PeerList({ peers, channel, onJoin, onTalk }: { peers: PeerInfo[]; channel: number; onJoin: (ch:number)=>void; onTalk: ()=>void }) {
  return (
    <div className="peer-list">
      {peers.map((peer) => (
        <div key={peer.device_id} className={`peer-card ${peer.channel === channel ? 'same-channel' : ''}`}>
          <div className="peer-main">
            <UserAvatar
              deviceId={peer.device_id}
              deviceName={peer.device_name}
              size={44}
              showStatus={true}
              status={peer.channel === channel ? 'online' : 'away'}
              platform={undefined}
            />
            <div className="peer-details">
              <span className="peer-name">{peer.device_name}</span>
              <span className="peer-meta">
                <span>CH{peer.channel.toString().padStart(2, '0')}</span>
                <IconSignal size={16} level={peer.last_seen ? getSignalLevel(peer.last_seen * 1000) : 0} />
              </span>
            </div>
          </div>
          <div className="peer-actions">
            {peer.channel === channel ? (
              <button className="connected-btn" onClick={onTalk}>Talk</button>
            ) : (
              <button className="connect-btn" onClick={() => onJoin(peer.channel)}>
                Join CH{peer.channel.toString().padStart(2, '0')}
              </button>
            )}
          </div>
        </div>
      ))}
    </div>
  );
}
