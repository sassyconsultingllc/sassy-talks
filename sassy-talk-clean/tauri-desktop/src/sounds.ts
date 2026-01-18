// sounds.ts - Audio Feedback System
// Copyright 2025 Sassy Consulting LLC. All rights reserved.
// Generates tones using Web Audio API

let audioContext: AudioContext | null = null;

function getAudioContext(): AudioContext {
  if (!audioContext) {
    audioContext = new AudioContext();
  }
  // Resume if suspended (browser autoplay policy)
  if (audioContext.state === 'suspended') {
    audioContext.resume();
  }
  return audioContext;
}

/**
 * Play a single tone
 */
function playTone(
  frequency: number,
  duration: number,
  startTime: number,
  volume: number = 0.3,
  type: OscillatorType = 'sine'
): void {
  const ctx = getAudioContext();
  
  const oscillator = ctx.createOscillator();
  const gainNode = ctx.createGain();
  
  oscillator.type = type;
  oscillator.frequency.value = frequency;
  
  // Envelope for smooth attack/release
  gainNode.gain.setValueAtTime(0, startTime);
  gainNode.gain.linearRampToValueAtTime(volume, startTime + 0.02); // 20ms attack
  gainNode.gain.setValueAtTime(volume, startTime + duration - 0.05);
  gainNode.gain.linearRampToValueAtTime(0, startTime + duration); // 50ms release
  
  oscillator.connect(gainNode);
  gainNode.connect(ctx.destination);
  
  oscillator.start(startTime);
  oscillator.stop(startTime + duration);
}

/**
 * Three-tone connection success chime (Windows XP style)
 * Rising major chord: C5 → E5 → G5 with overlap
 */
export function playConnectionSuccess(): void {
  const ctx = getAudioContext();
  const now = ctx.currentTime;
  
  // Frequencies for a pleasant rising chime (C major chord)
  const tones = [
    { freq: 523.25, start: 0, duration: 0.25 },      // C5
    { freq: 659.25, start: 0.12, duration: 0.25 },   // E5 (overlaps)
    { freq: 783.99, start: 0.24, duration: 0.35 },   // G5 (overlaps, longer)
  ];
  
  tones.forEach(tone => {
    playTone(tone.freq, tone.duration, now + tone.start, 0.25, 'sine');
  });
}

/**
 * Two-tone message delivered (low → high)
 * 450Hz → 480Hz rising confirmation
 */
export function playMessageDelivered(): void {
  const ctx = getAudioContext();
  const now = ctx.currentTime;
  
  playTone(450, 0.12, now, 0.3, 'sine');
  playTone(480, 0.15, now + 0.1, 0.3, 'sine');
}

/**
 * Two-tone failure/error (monotone)
 * 330Hz → 330Hz like Windows error dialog
 */
export function playError(): void {
  const ctx = getAudioContext();
  const now = ctx.currentTime;
  
  playTone(330, 0.15, now, 0.35, 'square'); // Square wave for harsher tone
  playTone(330, 0.15, now + 0.2, 0.35, 'square');
}

/**
 * Roger beep - end of transmission
 * Classic short high-pitched beep
 */
export function playRogerBeep(): void {
  const ctx = getAudioContext();
  const now = ctx.currentTime;
  
  playTone(1200, 0.08, now, 0.2, 'sine');
  playTone(1000, 0.12, now + 0.06, 0.15, 'sine');
}

/**
 * PTT start beep - transmission starting
 * Quick ascending tone
 */
export function playPTTStart(): void {
  const ctx = getAudioContext();
  const now = ctx.currentTime;
  
  playTone(600, 0.06, now, 0.2, 'sine');
  playTone(800, 0.08, now + 0.04, 0.2, 'sine');
}

/**
 * Channel change confirmation
 * Quick blip
 */
export function playChannelChange(): void {
  const ctx = getAudioContext();
  const now = ctx.currentTime;
  
  playTone(880, 0.05, now, 0.15, 'sine');
}

/**
 * Incoming transmission alert
 * Two quick high tones
 */
export function playIncomingTransmission(): void {
  const ctx = getAudioContext();
  const now = ctx.currentTime;
  
  playTone(1000, 0.08, now, 0.2, 'sine');
  playTone(1200, 0.1, now + 0.1, 0.25, 'sine');
}

/**
 * Discovery started
 * Ascending sweep
 */
export function playDiscoveryStart(): void {
  const ctx = getAudioContext();
  const now = ctx.currentTime;
  
  const oscillator = ctx.createOscillator();
  const gainNode = ctx.createGain();
  
  oscillator.type = 'sine';
  oscillator.frequency.setValueAtTime(400, now);
  oscillator.frequency.linearRampToValueAtTime(800, now + 0.3);
  
  gainNode.gain.setValueAtTime(0, now);
  gainNode.gain.linearRampToValueAtTime(0.2, now + 0.05);
  gainNode.gain.setValueAtTime(0.2, now + 0.25);
  gainNode.gain.linearRampToValueAtTime(0, now + 0.3);
  
  oscillator.connect(gainNode);
  gainNode.connect(ctx.destination);
  
  oscillator.start(now);
  oscillator.stop(now + 0.3);
}

/**
 * Discovery stopped
 * Descending sweep
 */
export function playDiscoveryStop(): void {
  const ctx = getAudioContext();
  const now = ctx.currentTime;
  
  const oscillator = ctx.createOscillator();
  const gainNode = ctx.createGain();
  
  oscillator.type = 'sine';
  oscillator.frequency.setValueAtTime(800, now);
  oscillator.frequency.linearRampToValueAtTime(400, now + 0.25);
  
  gainNode.gain.setValueAtTime(0, now);
  gainNode.gain.linearRampToValueAtTime(0.2, now + 0.05);
  gainNode.gain.setValueAtTime(0.2, now + 0.2);
  gainNode.gain.linearRampToValueAtTime(0, now + 0.25);
  
  oscillator.connect(gainNode);
  gainNode.connect(ctx.destination);
  
  oscillator.start(now);
  oscillator.stop(now + 0.25);
}

// Export all sounds as a namespace for convenience
export const Sounds = {
  connectionSuccess: playConnectionSuccess,
  messageDelivered: playMessageDelivered,
  error: playError,
  rogerBeep: playRogerBeep,
  pttStart: playPTTStart,
  channelChange: playChannelChange,
  incomingTransmission: playIncomingTransmission,
  discoveryStart: playDiscoveryStart,
  discoveryStop: playDiscoveryStop,
};

export default Sounds;
