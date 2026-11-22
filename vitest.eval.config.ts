import { defineConfig } from 'vitest/config';

// Conversation evaluation (Phase 4): deterministic simulated scenarios.
export default defineConfig({
  test: {
    environment: 'jsdom',
    include: ['eval/conversation/**/*.eval.ts', 'eval/intelligence/**/*.eval.ts', 'eval/local/**/*.eval.ts'],
    setupFiles: ['src/test/setup.ts'],
    testTimeout: 60000,
  },
});
