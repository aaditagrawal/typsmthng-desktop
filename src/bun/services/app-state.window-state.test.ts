import { afterEach, describe, expect, it } from 'vitest'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'

import { AppStateService } from './app-state'

describe('AppStateService window state without version.json', () => {
  const previousHome = process.env.HOME
  const previousXdg = process.env.XDG_DATA_HOME
  let home = ''

  afterEach(() => {
    if (previousHome === undefined) delete process.env.HOME
    else process.env.HOME = previousHome
    if (previousXdg === undefined) delete process.env.XDG_DATA_HOME
    else process.env.XDG_DATA_HOME = previousXdg
    if (home) rmSync(home, { recursive: true, force: true })
  })

  it('load() succeeds when Electrobun version.json is missing', async () => {
    home = mkdtempSync(path.join(tmpdir(), 'typsmthng-app-state-'))
    process.env.HOME = home
    delete process.env.XDG_DATA_HOME

    const service = new AppStateService()
    const metadata = await service.load()
    expect(metadata.windowState?.width).toBeGreaterThan(0)
    expect(metadata.windowState?.height).toBeGreaterThan(0)

    await service.setWindowState({
      x: 12,
      y: 34,
      width: 1280,
      height: 720,
    })
    const again = new AppStateService()
    const restored = await again.load()
    expect(restored.windowState).toEqual({
      x: 12,
      y: 34,
      width: 1280,
      height: 720,
    })
  })
})
