import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import './SettingsPanel.css';
import { IconSettings, IconClose } from './Icons';

interface AudioDeviceInfo {
  name: string;
  is_default: boolean;
  device_type: string;
}

interface AudioDevices {
  inputs: AudioDeviceInfo[];
  outputs: AudioDeviceInfo[];
}

interface SettingsPanelProps {
  onClose: () => void;
}

function SettingsPanel({ onClose }: SettingsPanelProps) {
  const [audioDevices, setAudioDevices] = useState<AudioDevices | null>(null);
  const [selectedInput, setSelectedInput] = useState<string>('default');
  const [selectedOutput, setSelectedOutput] = useState<string>('default');
  const [inputVolume, setInputVolume] = useState(100);
  const [outputVolume, setOutputVolume] = useState(100);
  const [rogerBeep, setRogerBeep] = useState(true);
  const [voxEnabled, setVoxEnabled] = useState(false);
  const [voxThreshold, setVoxThreshold] = useState(10);

  useEffect(() => {
    loadSettings();
  }, []);

  const loadSettings = async () => {
    try {
      const devices = await invoke<AudioDevices>('get_audio_devices');
      setAudioDevices(devices);
      
      const volume = await invoke<{ input: number; output: number }>('get_volume');
      setInputVolume(Math.round(volume.input * 100));
      setOutputVolume(Math.round(volume.output * 100));
    } catch (err) {
      console.error('Failed to load settings:', err);
    }
  };

  const handleInputDeviceChange = async (deviceName: string) => {
    try {
      await invoke('set_input_device', { deviceName });
      setSelectedInput(deviceName);
    } catch (err) {
      console.error('Failed to set input device:', err);
    }
  };

  const handleOutputDeviceChange = async (deviceName: string) => {
    try {
      await invoke('set_output_device', { deviceName });
      setSelectedOutput(deviceName);
    } catch (err) {
      console.error('Failed to set output device:', err);
    }
  };

  const handleVolumeChange = async (input: number, output: number) => {
    try {
      await invoke('set_volume', { 
        input: input / 100, 
        output: output / 100 
      });
    } catch (err) {
      console.error('Failed to set volume:', err);
    }
  };

  const handleInputVolumeChange = (value: number) => {
    setInputVolume(value);
    handleVolumeChange(value, outputVolume);
  };

  const handleOutputVolumeChange = (value: number) => {
    setOutputVolume(value);
    handleVolumeChange(inputVolume, value);
  };

  const handleRogerBeepChange = async (enabled: boolean) => {
    try {
      await invoke('set_roger_beep', { enabled });
      setRogerBeep(enabled);
    } catch (err) {
      console.error('Failed to set roger beep:', err);
    }
  };

  const handleVoxEnabledChange = async (enabled: boolean) => {
    try {
      await invoke('set_vox_enabled', { enabled });
      setVoxEnabled(enabled);
    } catch (err) {
      console.error('Failed to set VOX:', err);
    }
  };

  const handleVoxThresholdChange = async (threshold: number) => {
    try {
      await invoke('set_vox_threshold', { threshold: threshold / 100 });
      setVoxThreshold(threshold);
    } catch (err) {
      console.error('Failed to set VOX threshold:', err);
    }
  };

  return (
    <div className="settings-overlay" onClick={onClose}>
      <div className="settings-panel" onClick={(e) => e.stopPropagation()}>
        <div className="settings-header">
          <h2><IconSettings size={20} /> Settings</h2>
          <button className="close-button" onClick={onClose}><IconClose size={20} /></button>
        </div>

        <div className="settings-content">
          {/* Audio Devices */}
          <section className="settings-section">
            <h3>Audio Devices</h3>
            
            <div className="setting-item">
              <label>Microphone</label>
              <select 
                value={selectedInput}
                onChange={(e) => handleInputDeviceChange(e.target.value)}
              >
                <option value="default">Default</option>
                {audioDevices?.inputs.map(device => (
                  <option key={device.name} value={device.name}>
                    {device.name}
                  </option>
                ))}
              </select>
            </div>

            <div className="setting-item">
              <label>Speaker</label>
              <select 
                value={selectedOutput}
                onChange={(e) => handleOutputDeviceChange(e.target.value)}
              >
                <option value="default">Default</option>
                {audioDevices?.outputs.map(device => (
                  <option key={device.name} value={device.name}>
                    {device.name}
                  </option>
                ))}
              </select>
            </div>
          </section>

          {/* Volume */}
          <section className="settings-section">
            <h3>Volume</h3>
            
            <div className="setting-item">
              <label>Microphone Volume</label>
              <div className="volume-control">
                <input 
                  type="range" 
                  min="0" 
                  max="200"
                  value={inputVolume}
                  onChange={(e) => handleInputVolumeChange(parseInt(e.target.value))}
                />
                <span className="volume-value">{inputVolume}%</span>
              </div>
            </div>

            <div className="setting-item">
              <label>Speaker Volume</label>
              <div className="volume-control">
                <input 
                  type="range" 
                  min="0" 
                  max="200"
                  value={outputVolume}
                  onChange={(e) => handleOutputVolumeChange(parseInt(e.target.value))}
                />
                <span className="volume-value">{outputVolume}%</span>
              </div>
            </div>
          </section>

          {/* PTT Options */}
          <section className="settings-section">
            <h3>PTT Options</h3>
            
            <div className="setting-item">
              <label>
                <input 
                  type="checkbox" 
                  checked={rogerBeep}
                  onChange={(e) => handleRogerBeepChange(e.target.checked)}
                />
                Roger Beep
              </label>
              <p className="setting-description">
                Play a beep sound when you release PTT
              </p>
            </div>

            <div className="setting-item">
              <label>
                <input 
                  type="checkbox" 
                  checked={voxEnabled}
                  onChange={(e) => handleVoxEnabledChange(e.target.checked)}
                />
                Voice Activation (VOX)
              </label>
              <p className="setting-description">
                Automatically transmit when you speak
              </p>
            </div>

            {voxEnabled && (
              <div className="setting-item">
                <label>VOX Sensitivity</label>
                <div className="volume-control">
                  <input 
                    type="range" 
                    min="1" 
                    max="100"
                    value={voxThreshold}
                    onChange={(e) => handleVoxThresholdChange(parseInt(e.target.value))}
                  />
                  <span className="volume-value">{voxThreshold}%</span>
                </div>
              </div>
            )}
          </section>

          {/* About */}
          <section className="settings-section">
            <h3>About</h3>
            <div className="about-info">
              <p><strong>Sassy-Talk Desktop</strong></p>
              <p>Version 1.0.0</p>
              <p>© 2025 Sassy Consulting LLC</p>
              <p className="about-description">
                Cross-platform PTT walkie-talkie with retro vibes
              </p>
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}

export default SettingsPanel;
