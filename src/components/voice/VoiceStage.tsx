import { SujitWaveform } from './SujitWaveform';
import { useRecognizerLevel } from './useRecognizerLevel';
import { cn } from '../../utils';

interface VoiceStageProps {
  capturing: boolean;
  hearing: boolean;
  partial: string;
  /** Paused/stopped: dim it. */
  settled?: boolean;
  /** Shorter once the transcript has content. */
  compact?: boolean;
  className?: string;
}

/** The Sujit wave as the stage: centred, full width, driven only by real recognizer events. */
export function VoiceStage({ capturing, hearing, partial, settled, compact, className }: VoiceStageProps) {
  const level = useRecognizerLevel(capturing, hearing, partial);
  const label = !capturing ? 'Voice wave: not listening' : hearing || partial ? 'Voice wave: speech detected' : 'Voice wave: listening, no speech right now';
  return (
    <div
      data-testid="voice-wave"
      data-capturing={capturing ? 'true' : 'false'}
      className={cn('relative transition-[height,opacity] duration-500 ease-[var(--ease-out)]', compact ? 'h-[136px]' : 'h-[clamp(200px,36vh,340px)]', settled && 'opacity-60', className)}
    >
      <SujitWaveform level={level} active={capturing} label={label} className="h-full" />
    </div>
  );
}
