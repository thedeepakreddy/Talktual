import { describe, it, expect } from 'vitest';
import { ActivityLevel } from './activityLevel';

describe('ActivityLevel (voice wave input)', () => {
  it('is flat with no recognizer events — nothing simulated', () => {
    let t = 0;
    const l = new ActivityLevel(650, () => t);
    expect(l.sample()).toBe(0);
    t = 10_000;
    expect(l.sample()).toBe(0);
  });

  it('rises on new interim words and decays back to flat in silence', () => {
    let t = 0;
    const l = new ActivityLevel(650, () => t);
    l.bump(12);
    expect(l.sample()).toBeGreaterThan(0.5);
    t = 4000;
    expect(l.sample()).toBeLessThan(0.01);
  });

  it('holds a floor while the recognizer hears a voice; reset clears everything', () => {
    let t = 0;
    const l = new ActivityLevel(650, () => t);
    l.setHearing(true);
    t = 5000;
    expect(l.sample()).toBeCloseTo(0.22);
    l.reset();
    expect(l.sample()).toBe(0);
  });

  it('a real measured level takes over while it keeps reporting', () => {
    let t = 0;
    const l = new ActivityLevel(650, () => t);
    l.setDirect(0.8);
    expect(l.sample()).toBe(0.8);
    t = 1000; // stream went quiet / stopped reporting
    expect(l.sample()).toBe(0);
  });

  it('busier speech means more waves', () => {
    let t = 0;
    const l = new ActivityLevel(650, () => t);
    expect(l.busyness()).toBe(0);
    for (let i = 0; i < 8; i++) {
      t += 100;
      l.bump(3);
    }
    expect(l.busyness()).toBe(1);
  });
});
