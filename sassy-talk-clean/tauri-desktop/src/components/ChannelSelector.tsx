import './ChannelSelector.css';

interface ChannelSelectorProps {
  channel: number;
  onChange: (channel: number) => void;
}

function ChannelSelector({ channel, onChange }: ChannelSelectorProps) {
  const handlePrevious = () => {
    const newChannel = channel > 1 ? channel - 1 : 16;
    onChange(newChannel);
  };

  const handleNext = () => {
    const newChannel = channel < 16 ? channel + 1 : 1;
    onChange(newChannel);
  };

  const handleInput = (e: React.ChangeEvent<HTMLInputElement>) => {
    const value = parseInt(e.target.value);
    if (value >= 1 && value <= 16) {
      onChange(value);
    }
  };

  return (
    <div className="channel-selector">
      <label className="channel-label">Channel</label>
      <div className="channel-controls">
        <button 
          className="channel-button"
          onClick={handlePrevious}
          title="Previous channel"
        >
          ◀
        </button>
        <input
          type="number"
          className="channel-input"
          value={channel}
          onChange={handleInput}
          min="1"
          max="16"
        />
        <button 
          className="channel-button"
          onClick={handleNext}
          title="Next channel"
        >
          ▶
        </button>
      </div>
    </div>
  );
}

export default ChannelSelector;
