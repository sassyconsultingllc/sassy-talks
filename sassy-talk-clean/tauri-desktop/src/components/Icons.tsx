// Icons.tsx - SVG icon components (no emoji)
// Copyright 2025 Sassy Consulting LLC. All rights reserved.

import { CSSProperties } from 'react';

interface IconProps {
  size?: number;
  color?: string;
  className?: string;
  style?: CSSProperties;
}

// Navigation & UI Icons
export const IconLobby = ({ size = 24, color = 'currentColor', className, style }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" className={className} style={style}>
    <circle cx="12" cy="12" r="3" stroke={color} strokeWidth="2"/>
    <path d="M12 2v4M12 18v4M2 12h4M18 12h4" stroke={color} strokeWidth="2" strokeLinecap="round"/>
    <path d="M4.93 4.93l2.83 2.83M16.24 16.24l2.83 2.83M4.93 19.07l2.83-2.83M16.24 7.76l2.83-2.83" stroke={color} strokeWidth="2" strokeLinecap="round"/>
  </svg>
);

export const IconRadio = ({ size = 24, color = 'currentColor', className, style }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" className={className} style={style}>
    <rect x="4" y="6" width="16" height="14" rx="2" stroke={color} strokeWidth="2"/>
    <circle cx="9" cy="14" r="2.5" stroke={color} strokeWidth="2"/>
    <path d="M14 11h4M14 14h4M14 17h2" stroke={color} strokeWidth="2" strokeLinecap="round"/>
    <path d="M8 2l4 4 4-4" stroke={color} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"/>
  </svg>
);

export const IconSettings = ({ size = 24, color = 'currentColor', className, style }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" className={className} style={style}>
    <circle cx="12" cy="12" r="3" stroke={color} strokeWidth="2"/>
    <path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-2 2 2 2 0 01-2-2v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83 0 2 2 0 010-2.83l.06-.06a1.65 1.65 0 00.33-1.82 1.65 1.65 0 00-1.51-1H3a2 2 0 01-2-2 2 2 0 012-2h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 010-2.83 2 2 0 012.83 0l.06.06a1.65 1.65 0 001.82.33H9a1.65 1.65 0 001-1.51V3a2 2 0 012-2 2 2 0 012 2v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 0 2 2 0 010 2.83l-.06.06a1.65 1.65 0 00-.33 1.82V9a1.65 1.65 0 001.51 1H21a2 2 0 012 2 2 2 0 01-2 2h-.09a1.65 1.65 0 00-1.51 1z" stroke={color} strokeWidth="2"/>
  </svg>
);

export const IconSearch = ({ size = 24, color = 'currentColor', className, style }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" className={className} style={style}>
    <circle cx="11" cy="11" r="7" stroke={color} strokeWidth="2"/>
    <path d="M21 21l-4.35-4.35" stroke={color} strokeWidth="2" strokeLinecap="round"/>
  </svg>
);

export const IconClose = ({ size = 24, color = 'currentColor', className, style }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" className={className} style={style}>
    <path d="M18 6L6 18M6 6l12 12" stroke={color} strokeWidth="2" strokeLinecap="round"/>
  </svg>
);

export const IconRefresh = ({ size = 24, color = 'currentColor', className, style }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" className={className} style={style}>
    <path d="M1 4v6h6M23 20v-6h-6" stroke={color} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"/>
    <path d="M20.49 9A9 9 0 005.64 5.64L1 10m22 4l-4.64 4.36A9 9 0 013.51 15" stroke={color} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"/>
  </svg>
);

export const IconLock = ({ size = 24, color = 'currentColor', className, style }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" className={className} style={style}>
    <rect x="3" y="11" width="18" height="11" rx="2" stroke={color} strokeWidth="2"/>
    <path d="M7 11V7a5 5 0 0110 0v4" stroke={color} strokeWidth="2" strokeLinecap="round"/>
  </svg>
);

export const IconChevronLeft = ({ size = 24, color = 'currentColor', className, style }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" className={className} style={style}>
    <path d="M15 18l-6-6 6-6" stroke={color} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"/>
  </svg>
);

export const IconChevronRight = ({ size = 24, color = 'currentColor', className, style }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" className={className} style={style}>
    <path d="M9 18l6-6-6-6" stroke={color} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"/>
  </svg>
);

// Audio Icons
export const IconMic = ({ size = 24, color = 'currentColor', className, style }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" className={className} style={style}>
    <rect x="9" y="2" width="6" height="12" rx="3" stroke={color} strokeWidth="2"/>
    <path d="M5 10v1a7 7 0 0014 0v-1M12 19v3M8 22h8" stroke={color} strokeWidth="2" strokeLinecap="round"/>
  </svg>
);

export const IconMicFilled = ({ size = 24, color = 'currentColor', className, style }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill={color} className={className} style={style}>
    <rect x="9" y="2" width="6" height="12" rx="3"/>
    <path d="M5 10v1a7 7 0 0014 0v-1" stroke={color} strokeWidth="2" strokeLinecap="round" fill="none"/>
    <path d="M12 19v3M8 22h8" stroke={color} strokeWidth="2" strokeLinecap="round" fill="none"/>
  </svg>
);

export const IconSpeaker = ({ size = 24, color = 'currentColor', className, style }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" className={className} style={style}>
    <path d="M11 5L6 9H2v6h4l5 4V5z" stroke={color} strokeWidth="2" strokeLinejoin="round"/>
    <path d="M15.54 8.46a5 5 0 010 7.07M19.07 4.93a10 10 0 010 14.14" stroke={color} strokeWidth="2" strokeLinecap="round"/>
  </svg>
);

export const IconRecording = ({ size = 24, color = '#ef4444', className, style }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" className={className} style={style}>
    <circle cx="12" cy="12" r="8" fill={color}/>
    <circle cx="12" cy="12" r="10" stroke={color} strokeWidth="2" opacity="0.5"/>
  </svg>
);

// Signal Strength (1-5 bars)
export const IconSignal = ({ size = 24, color = 'currentColor', level = 5, className, style }: IconProps & { level?: number }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" className={className} style={style}>
    <rect x="2" y="16" width="3" height="6" rx="1" fill={level >= 1 ? color : 'rgba(128,128,128,0.3)'}/>
    <rect x="7" y="12" width="3" height="10" rx="1" fill={level >= 2 ? color : 'rgba(128,128,128,0.3)'}/>
    <rect x="12" y="8" width="3" height="14" rx="1" fill={level >= 3 ? color : 'rgba(128,128,128,0.3)'}/>
    <rect x="17" y="4" width="3" height="18" rx="1" fill={level >= 4 ? color : 'rgba(128,128,128,0.3)'}/>
    <rect x="22" y="0" width="0" height="0" fill="none"/>
  </svg>
);

// Platform Icons
export const IconAndroid = ({ size = 24, color = 'currentColor', className, style }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill={color} className={className} style={style}>
    <path d="M6 18c0 .55.45 1 1 1h1v3.5c0 .83.67 1.5 1.5 1.5s1.5-.67 1.5-1.5V19h2v3.5c0 .83.67 1.5 1.5 1.5s1.5-.67 1.5-1.5V19h1c.55 0 1-.45 1-1V8H6v10zM3.5 8C2.67 8 2 8.67 2 9.5v7c0 .83.67 1.5 1.5 1.5S5 17.33 5 16.5v-7C5 8.67 4.33 8 3.5 8zm17 0c-.83 0-1.5.67-1.5 1.5v7c0 .83.67 1.5 1.5 1.5s1.5-.67 1.5-1.5v-7c0-.83-.67-1.5-1.5-1.5zm-4.97-5.84l1.3-1.3c.2-.2.2-.51 0-.71-.2-.2-.51-.2-.71 0l-1.48 1.48A5.93 5.93 0 0012 1c-.96 0-1.86.23-2.66.63L7.85.15c-.2-.2-.51-.2-.71 0-.2.2-.2.51 0 .71l1.31 1.31A5.96 5.96 0 006 7h12c0-1.99-.97-3.75-2.47-4.84zM10 5H9V4h1v1zm5 0h-1V4h1v1z"/>
  </svg>
);

export const IconApple = ({ size = 24, color = 'currentColor', className, style }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill={color} className={className} style={style}>
    <path d="M18.71 19.5c-.83 1.24-1.71 2.45-3.05 2.47-1.34.03-1.77-.79-3.29-.79-1.53 0-2 .77-3.27.82-1.31.05-2.3-1.32-3.14-2.53C4.25 17 2.94 12.45 4.7 9.39c.87-1.52 2.43-2.48 4.12-2.51 1.28-.02 2.5.87 3.29.87.78 0 2.26-1.07 3.81-.91.65.03 2.47.26 3.64 1.98-.09.06-2.17 1.28-2.15 3.81.03 3.02 2.65 4.03 2.68 4.04-.03.07-.42 1.44-1.38 2.83M13 3.5c.73-.83 1.94-1.46 2.94-1.5.13 1.17-.34 2.35-1.04 3.19-.69.85-1.83 1.51-2.95 1.42-.15-1.15.41-2.35 1.05-3.11z"/>
  </svg>
);

export const IconWindows = ({ size = 24, color = 'currentColor', className, style }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill={color} className={className} style={style}>
    <path d="M0 3.449L9.75 2.1v9.451H0m10.949-9.602L24 0v11.4H10.949M0 12.6h9.75v9.451L0 20.699M10.949 12.6H24V24l-12.9-1.801"/>
  </svg>
);

export const IconLinux = ({ size = 24, color = 'currentColor', className, style }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill={color} className={className} style={style}>
    <path d="M12.504 0c-.155 0-.315.008-.48.021-4.226.333-3.105 4.807-3.17 6.298-.076 1.092-.3 1.953-1.05 3.02-.885 1.051-2.127 2.75-2.716 4.521-.278.832-.41 1.684-.287 2.489.117.78.442 1.516 1.053 2.136.67.68 1.586 1.208 2.654 1.53.832.252 1.71.376 2.602.376.893 0 1.77-.124 2.603-.377 1.068-.321 1.984-.85 2.654-1.53.61-.619.936-1.355 1.053-2.135.123-.805-.01-1.657-.287-2.49-.59-1.77-1.831-3.469-2.717-4.52-.75-1.067-.974-1.928-1.05-3.02-.064-1.491 1.057-5.965-3.17-6.298-.164-.013-.324-.021-.479-.021zm1.45 4.714c.228.23.415.55.53.891.115.341.166.703.166 1.014 0 .311-.05.622-.132.871-.082.248-.189.446-.3.583a.62.62 0 01-.108.11c-.13.076-.3.13-.489.166a1.82 1.82 0 01-.494 0c-.189-.036-.359-.09-.489-.166a.615.615 0 01-.108-.11c-.11-.137-.218-.335-.3-.583-.082-.25-.132-.56-.132-.871 0-.31.05-.673.166-1.014.115-.341.302-.66.53-.891.228-.23.494-.368.78-.368.287 0 .553.138.78.368z"/>
  </svg>
);

export const IconDevice = ({ size = 24, color = 'currentColor', className, style }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="none" className={className} style={style}>
    <rect x="5" y="2" width="14" height="20" rx="2" stroke={color} strokeWidth="2"/>
    <circle cx="12" cy="18" r="1" fill={color}/>
    <path d="M9 5h6" stroke={color} strokeWidth="2" strokeLinecap="round"/>
  </svg>
);

// Get platform icon component by name
export const getPlatformIcon = (platform: string | undefined, size = 20, color = 'currentColor') => {
  switch (platform) {
    case 'Android': return <IconAndroid size={size} color={color} />;
    case 'iOS': 
    case 'MacOS': return <IconApple size={size} color={color} />;
    case 'Windows': return <IconWindows size={size} color={color} />;
    case 'Linux': return <IconLinux size={size} color={color} />;
    default: return <IconDevice size={size} color={color} />;
  }
};

// Signal level helper (converts timestamp age to 1-5)
export const getSignalLevel = (lastSeen: number): number => {
  const age = Date.now() - lastSeen;
  if (age < 1000) return 5;
  if (age < 3000) return 4;
  if (age < 5000) return 3;
  if (age < 10000) return 2;
  return 1;
};
