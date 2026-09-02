import { describe, expect, it } from 'vitest'
import path from 'node:path'
import { isSystemLinuxInstall, resolvePackagedAppRoot, windowsTypstOpenCommand } from './platform-setup'

describe('windowsTypstOpenCommand', () => {
  it('keeps %1 literal so cmd.exe cannot eat the file-association placeholder', () => {
    expect(windowsTypstOpenCommand('C:\\Apps\\launcher.exe')).toBe('"C:\\Apps\\launcher.exe" "%1"')
  })
})

describe('resolvePackagedAppRoot', () => {
  it('returns the directory that contains Resources/version.json', () => {
    const execPath = path.join('/opt', 'typsmthng', 'bin', 'launcher')
    const exists = (candidate: string) =>
      candidate === path.join('/opt', 'typsmthng', 'Resources', 'version.json')
    expect(resolvePackagedAppRoot(execPath, exists)).toBe(path.join('/opt', 'typsmthng'))
  })

  it('returns null in unpackaged / dev layouts', () => {
    expect(resolvePackagedAppRoot('/usr/bin/bun', () => false)).toBeNull()
  })
})

describe('isSystemLinuxInstall', () => {
  it('detects deb/rpm install prefixes', () => {
    expect(isSystemLinuxInstall('/opt/typsmthng/bin/launcher')).toBe(true)
    expect(isSystemLinuxInstall('/usr/bin/typsmthng')).toBe(true)
    expect(isSystemLinuxInstall('/usr/lib/typsmthng/bin/launcher')).toBe(true)
  })

  it('ignores AppImage and ad-hoc layouts', () => {
    expect(isSystemLinuxInstall('/home/user/Downloads/typsmthng.AppImage')).toBe(false)
    expect(isSystemLinuxInstall('/tmp/AppDir/usr/typsmthng/bin/launcher')).toBe(false)
    expect(isSystemLinuxInstall('/usr/bin/bun')).toBe(false)
  })
})
