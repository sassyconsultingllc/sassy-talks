import { useState, useEffect } from 'react';
import './PTTButton.css';

interface PTTButtonProps {
  isTransmitting: boolean;
  isConnected: boolean;
  onPress: () => void;
  onRelease: () => void;
}

function PTTButton({ isTransmitting, isConnected, onPress, onRelease }: PTTButtonProps) {
  const [isPressed, setIsPressed] = useState(false);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.code === 'Space' && !isPressed && isConnected) {
        e.preventDefault();
        setIsPressed(true);
        onPress();
      }
    };

    const handleKeyUp = (e: KeyboardEvent) => {
      if (e.code === 'Space' && isPressed) {
        e.preventDefault();
        setIsPressed(false);
        onRelease();
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    window.addEventListener('keyup', handleKeyUp);

    return () => {
      window.removeEventListener('keydown', handleKeyDown);
      window.removeEventListener('keyup', handleKeyUp);
    };
  }, [isPressed, isConnected, onPress, onRelease]);

  const handleMouseDown = () => {
    if (!isConnected) return;
    setIsPressed(true);
    onPress();
  };

  const handleMouseUp = () => {
    if (!isPressed) return;
    setIsPressed(false);
    onRelease();
  };

  const handleTouchStart = (e: React.TouchEvent) => {
    e.preventDefault();
    if (!isConnected) return;
    setIsPressed(true);
    onPress();
  };

  const handleTouchEnd = (e: React.TouchEvent) => {
    e.preventDefault();
    if (!isPressed) return;
    setIsPressed(false);
    onRelease();
  };

  const buttonClass = `ptt-button ${isTransmitting ? 'transmitting' : ''} ${!isConnected ? 'disabled' : ''}`;

  return (
    <div className="ptt-container">
      <button
        className={buttonClass}
        onMouseDown={handleMouseDown}
        onMouseUp={handleMouseUp}
        onMouseLeave={handleMouseUp}
        onTouchStart={handleTouchStart}
        onTouchEnd={handleTouchEnd}
        disabled={!isConnected}
      >
        <div className="ptt-icon">
          {isTransmitting ? '🎙️' : '🔇'}
        </div>
        <div className="ptt-label">
          {isTransmitting ? 'TRANSMITTING' : 'PUSH TO TALK'}
        </div>
      </button>
      <div className="ptt-hint">
        {isConnected ? 'Hold to talk (or press Space)' : 'Connecting...'}
      </div>
    </div>
  );
}

export default PTTButton;
