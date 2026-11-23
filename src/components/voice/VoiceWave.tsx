import { SujitWaveform } from './SujitWaveform';
import { useRecognizerLevel } from './useRecognizerLevel';
import { cn } from '../../utils';

interface VoiceWaveProps {
  capturing: boolean;
  hearing: boolean;
  partial: string;
  /** Paused/stopped: dim the idle line. */
  settled?: boolean;
  /** Real loudness from an open microphone stream, where the app holds one. */
  meter?: (onLevel: (v: number) => void) => (() => void) | null;
  className?: string;
}

/** The voice wave for AIR and calls, driven only by real capture signals. */
export function VoiceWave({ capturing, hearing, partial, settled, meter, className }: VoiceWaveProps) {
  const level = useRecognizerLevel(capturing, hearing, partial, meter);
  const label = !capturing ? 'Voice wave: not listening' : hearing || partial ? 'Voice wave: speech detected' : 'Voice wave: listening, no speech right now';
  return (
    <div data-testid="voice-wave" data-capturing={capturing ? 'true' : 'false'} className={cn('transition-opacity duration-200', settled && 'opacity-45', className)}>
      <SujitWaveform level={level} active={capturing} label={label} className="h-full" />
    </div>
  );
}
