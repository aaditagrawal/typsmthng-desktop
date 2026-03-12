import { vi } from 'vitest'

const classList = {
  toggle: vi.fn(),
  add: vi.fn(),
  remove: vi.fn(),
  contains: vi.fn(() => false),
}

Object.defineProperty(globalThis, 'document', {
  configurable: true,
  value: {
    documentElement: {
      classList,
    },
  },
})

Object.defineProperty(globalThis, 'window', {
  configurable: true,
  value: {
    matchMedia: vi.fn(() => ({
      matches: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
    localStorage: {
      getItem: vi.fn(() => null),
      setItem: vi.fn(),
      removeItem: vi.fn(),
    },
  },
})
