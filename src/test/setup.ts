import { afterEach } from 'vitest';
import { cleanup } from '@testing-library/react';

afterEach(() => cleanup());

// jsdom doesn't implement scrolling; the animation library probes it.
window.scrollTo = () => {};
