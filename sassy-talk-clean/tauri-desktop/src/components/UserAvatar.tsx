// UserAvatar.tsx - Generated avatar component based on device ID
// Copyright 2025 Sassy Consulting LLC. All rights reserved.

import { useMemo } from 'react';
import { IconAndroid, IconApple, IconWindows, IconLinux, IconDevice } from './Icons';

interface UserAvatarProps {
  deviceId: number | string;
  deviceName?: string;
  size?: number;
  showStatus?: boolean;
  status?: 'online' | 'offline' | 'busy' | 'away';
  platform?: 'Android' | 'iOS' | 'MacOS' | 'Windows' | 'Linux' | string;
  variant?: 'identicon' | 'letter' | 'gradient';
  className?: string;
}

// Generate consistent hash from device ID
function hashCode(str: string): number {
  let hash = 0;
  for (let i = 0; i < str.length; i++) {
    const char = str.charCodeAt(i);
    hash = ((hash << 5) - hash) + char;
    hash = hash & hash; // Convert to 32bit integer
  }
  return Math.abs(hash);
}

// Generate gradient pair
function hashToGradient(hash: number): [string, string] {
  const hue1 = hash % 360;
  const hue2 = (hash * 7) % 360;
  return [
    `hsl(${hue1}, 70%, 55%)`,
    `hsl(${hue2}, 70%, 45%)`
  ];
}

// Generate identicon SVG pattern
function generateIdenticon(hash: number, size: number): string {
  const grid = 5;
  const cellSize = size / grid;
  const cells: boolean[][] = [];
  
  // Generate symmetric pattern (mirror left to right)
  for (let y = 0; y < grid; y++) {
    cells[y] = [];
    for (let x = 0; x < Math.ceil(grid / 2); x++) {
      const bit = (hash >> (y * 3 + x)) & 1;
      cells[y][x] = bit === 1;
      cells[y][grid - 1 - x] = bit === 1; // Mirror
    }
  }
  
  // Build SVG path
  let path = '';
  for (let y = 0; y < grid; y++) {
    for (let x = 0; x < grid; x++) {
      if (cells[y][x]) {
        path += `M${x * cellSize},${y * cellSize}h${cellSize}v${cellSize}h-${cellSize}z`;
      }
    }
  }
  
  return path;
}

export default function UserAvatar({
  deviceId,
  deviceName = '',
  size = 40,
  showStatus = false,
  status = 'offline',
  platform,
  variant = 'identicon',
  className = ''
}: UserAvatarProps) {
  
  const idString = typeof deviceId === 'number' 
    ? deviceId.toString(16).padStart(8, '0') 
    : deviceId;
  
  const hash = useMemo(() => hashCode(idString), [idString]);
  const [gradStart, gradEnd] = useMemo(() => hashToGradient(hash), [hash]);
  const identiconPath = useMemo(() => generateIdenticon(hash, size * 0.7), [hash, size]);
  
  // Get initials from device name
  const initials = useMemo(() => {
    if (!deviceName) return '?';
    const words = deviceName.trim().split(/\s+/);
    if (words.length >= 2) {
      return (words[0][0] + words[1][0]).toUpperCase();
    }
    return deviceName.slice(0, 2).toUpperCase();
  }, [deviceName]);
  
  const statusColors = {
    online: '#4CD964',
    offline: '#8E8E93',
    busy: '#FF3B30',
    away: '#FFCC00'
  };
  
  const containerStyle: React.CSSProperties = {
    position: 'relative',
    width: size,
    height: size,
    borderRadius: '50%',
    overflow: 'hidden',
    flexShrink: 0,
    display: 'inline-flex',
    alignItems: 'center',
    justifyContent: 'center'
  };
  
  const statusDotStyle: React.CSSProperties = {
    position: 'absolute',
    bottom: 0,
    right: 0,
    width: size * 0.3,
    height: size * 0.3,
    borderRadius: '50%',
    backgroundColor: statusColors[status],
    border: `2px solid #1a1a2e`,
    boxSizing: 'border-box'
  };

  const platformBadgeStyle: React.CSSProperties = {
    position: 'absolute',
    bottom: -2,
    right: -2,
    width: size * 0.4,
    height: size * 0.4,
    borderRadius: '50%',
    backgroundColor: '#1a1a2e',
    border: '2px solid #25253e',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    boxSizing: 'border-box'
  };

  // Get platform icon component
  const getPlatformBadge = () => {
    if (!platform) return null;
    const iconSize = size * 0.22;
    const iconColor = gradStart;
    
    switch (platform) {
      case 'Android':
        return <IconAndroid size={iconSize} color={iconColor} />;
      case 'iOS':
      case 'MacOS':
        return <IconApple size={iconSize} color={iconColor} />;
      case 'Windows':
        return <IconWindows size={iconSize} color={iconColor} />;
      case 'Linux':
        return <IconLinux size={iconSize} color={iconColor} />;
      default:
        return <IconDevice size={iconSize} color={iconColor} />;
    }
  };

  // Render identicon variant
  if (variant === 'identicon') {
    return (
      <div className={`user-avatar ${className}`} style={containerStyle}>
        <svg 
          width={size} 
          height={size} 
          viewBox={`0 0 ${size} ${size}`}
          style={{ background: `linear-gradient(135deg, ${gradStart}, ${gradEnd})` }}
        >
          <defs>
            <linearGradient id={`grad-${idString}`} x1="0%" y1="0%" x2="100%" y2="100%">
              <stop offset="0%" stopColor={gradStart} />
              <stop offset="100%" stopColor={gradEnd} />
            </linearGradient>
          </defs>
          <rect width={size} height={size} fill={`url(#grad-${idString})`} />
          <g transform={`translate(${size * 0.15}, ${size * 0.15})`}>
            <path d={identiconPath} fill="rgba(255,255,255,0.9)" />
          </g>
        </svg>
        {platform && <div style={platformBadgeStyle}>{getPlatformBadge()}</div>}
        {showStatus && !platform && <div style={statusDotStyle} />}
      </div>
    );
  }
  
  // Render letter variant
  if (variant === 'letter') {
    return (
      <div 
        className={`user-avatar ${className}`} 
        style={{
          ...containerStyle,
          background: `linear-gradient(135deg, ${gradStart}, ${gradEnd})`,
          color: 'white',
          fontSize: size * 0.4,
          fontWeight: 600,
          fontFamily: '-apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
          textShadow: '0 1px 2px rgba(0,0,0,0.2)'
        }}
      >
        {initials}
        {platform && <div style={platformBadgeStyle}>{getPlatformBadge()}</div>}
        {showStatus && !platform && <div style={statusDotStyle} />}
      </div>
    );
  }
  
  // Render gradient variant (abstract)
  return (
    <div className={`user-avatar ${className}`} style={containerStyle}>
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
        <defs>
          <linearGradient id={`grad-${idString}`} x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stopColor={gradStart} />
            <stop offset="100%" stopColor={gradEnd} />
          </linearGradient>
          <radialGradient id={`shine-${idString}`} cx="30%" cy="30%" r="70%">
            <stop offset="0%" stopColor="rgba(255,255,255,0.3)" />
            <stop offset="100%" stopColor="rgba(255,255,255,0)" />
          </radialGradient>
        </defs>
        <circle cx={size/2} cy={size/2} r={size/2} fill={`url(#grad-${idString})`} />
        <circle cx={size/2} cy={size/2} r={size/2} fill={`url(#shine-${idString})`} />
        {/* Abstract pattern based on hash */}
        <g opacity="0.3">
          <circle 
            cx={size * 0.3 + (hash % 20)} 
            cy={size * 0.4} 
            r={size * 0.15} 
            fill="white" 
          />
          <circle 
            cx={size * 0.6 + ((hash >> 4) % 15)} 
            cy={size * 0.6} 
            r={size * 0.1} 
            fill="white" 
          />
        </g>
      </svg>
      {platform && <div style={platformBadgeStyle}>{getPlatformBadge()}</div>}
      {showStatus && !platform && <div style={statusDotStyle} />}
    </div>
  );
}

// Export utility for use elsewhere
export { hashCode, hashToGradient };
