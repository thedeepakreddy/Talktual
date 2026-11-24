import { useRecognizerLevel } from './useRecognizerLevel';
import { LogoWave } from '../../three/LogoWave';
import { cn } from '../../utils';

interface VoiceStageProps {
  capturing: boolean;
  hearing: boolean;
  partial: string;
  /** Paused/stopped: dim it. */
  settled?: boolean;
  /** hero: fills the middle of the screen; setup: large above the setup; compact: above the transcript. */
  size?: 'hero' | 'setup' | 'compact';
  className?: string;
}

/**
 * The Talktual mark as the voice stage: the logo's liquid-glass waves in
 * silence, a living waveform when speech is reported, back to the logo when it
 * stops. One level source, fed only by real recognizer events.
 */
export function VoiceStage({ capturing, hearing, partial, settled, size = 'hero', className }: VoiceStageProps) {
  const level = useRecognizerLevel(capturing, hearing, partial);
  const label = !capturing ? 'Voice: not listening' : hearing || partial ? 'Voice: speech detected' : 'Voice: listening, no speech right now';
  return (
    <div
      data-testid="voice-wave"
      data-capturing={capturing ? 'true' : 'false'}
      className={cn(
        'relative transition-[height,opacity] duration-500 ease-[var(--ease-out)]',
        size === 'hero' ? 'min-h-[240px] flex-1' : size === 'setup' ? 'h-[clamp(200px,34vh,340px)]' : 'h-[150px]',
        settled && 'opacity-70',
        className
      )}
    >
      <LogoWave level={level} active={capturing} label={label} className="absolute inset-0" />
    </div>
  );
}
