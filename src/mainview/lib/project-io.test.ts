import { describe, expect, it, vi, beforeEach } from 'vitest'
import { zipSync, unzipSync, strToU8, strFromU8 } from 'fflate'

const createProjectMock = vi.fn()
const downloadBlobMock = vi.fn()
const flushWritesMock = vi.fn().mockResolvedValue({ ok: true })
const getVaultExportBundleMock = vi.fn()

let mockProjects: Array<{
  id: string
  rootPath: string
  name: string
  files: unknown[]
  mainFile: string
}> = []

vi.mock('@/stores/project-store', () => ({
  clearSelectionState: vi.fn(),
  useProjectStore: Object.assign(
    vi.fn(),
    {
      getState: () => ({
        createProject: createProjectMock,
        getCurrentProject: () => null,
        projects: mockProjects,
      }),
      setState: vi.fn(),
    },
  ),
}))

vi.mock('@/lib/latex-converter', () => ({
  convertLatexToTypst: vi.fn(async (source: string) => {
    const title = source.match(/\\title\{([^}]+)\}/)?.[1]
    const author = source.match(/\\author\{([^}]+)\}/)?.[1]
    return {
      typst: title
        ? `#set document(\n  title: [${title}],\n)\n\n// converted\n${source}`
        : `// converted\n${source}`,
      warnings: [],
      metadata: { packages: [], graphicspath: [], title, author },
    }
  }),
}))

vi.mock('@/lib/desktop-rpc', () => ({
  desktopRpc: {
    request: {
      getVaultExportBundle: (...args: unknown[]) => getVaultExportBundleMock(...args),
      flushWrites: (...args: unknown[]) => flushWritesMock(...args),
    },
  },
}))

vi.mock('@/lib/download-blob', () => ({
  downloadBlob: (...args: unknown[]) => downloadBlobMock(...args),
}))

import {
  applyZipCommonRootStrip,
  exportAllProjects,
  exportProject,
  importAllProjects,
  importLatexProject,
  importLatexZip,
  importProject,
  normalizeImportEntryPath,
  resolveImportedMainFile,
  stripZipCommonRootPrefix,
  uniqueExportFolderName,
} from './project-io'

describe('resolveImportedMainFile', () => {
  it('prefers /main.typ when present', () => {
    expect(resolveImportedMainFile([
      { path: '/chapters/intro.typ' },
      { path: '/main.typ' },
    ])).toBe('/main.typ')
  })

  it('prefers a nested main.typ over the first zip entry', () => {
    expect(resolveImportedMainFile([
      { path: '/chapters/intro.typ' },
      { path: '/src/main.typ' },
    ])).toBe('/src/main.typ')
  })

  it('prefers Main.typ case-insensitively', () => {
    expect(resolveImportedMainFile([
      { path: '/lib.typ' },
      { path: '/Main.typ' },
    ])).toBe('/Main.typ')
  })

  it('prefers a nested main.typ over lib.typ', () => {
    expect(resolveImportedMainFile([
      { path: '/lib.typ' },
      { path: '/src/main.typ' },
    ])).toBe('/src/main.typ')
  })

  it('prefers the converted document over an abstract file', () => {
    expect(resolveImportedMainFile(
      [
        { path: '/abstract.typ', content: 'Short abstract' },
        { path: '/paper.typ', content: '#set document(\n  title: [Paper],\n)\n\nLong body\n' },
      ],
    )).toBe('/paper.typ')
  })

  it('honors documentPaths from \\begin{document} sources', () => {
    expect(resolveImportedMainFile(
      [
        { path: '/abstract.typ', content: 'abs' },
        { path: '/paper.typ', content: 'body' },
      ],
      { documentPaths: ['/paper.typ'] },
    )).toBe('/paper.typ')
  })

  it('falls back to the first .typ file', () => {
    expect(resolveImportedMainFile([
      { path: '/readme.md' },
      { path: '/paper.typ' },
    ])).toBe('/paper.typ')
  })
})

describe('stripZipCommonRootPrefix', () => {
  it('strips a shared Overleaf-style root folder', () => {
    expect(stripZipCommonRootPrefix([
      'Paper/main.tex',
      'Paper/refs.bib',
      'Paper/fig/a.png',
    ])).toBe('Paper')
  })

  it('does not strip when files sit at multiple roots', () => {
    expect(stripZipCommonRootPrefix([
      'Paper/main.tex',
      'Other/notes.typ',
    ])).toBe('')
  })

  it('does not strip when a file lives at the archive root', () => {
    expect(stripZipCommonRootPrefix([
      'main.tex',
      'Paper/refs.bib',
    ])).toBe('')
  })
})

describe('applyZipCommonRootStrip', () => {
  it('removes the common root and keeps a leading slash', () => {
    expect(applyZipCommonRootStrip('Paper/main.tex', 'Paper')).toBe('/main.tex')
  })

  it('normalizes paths when there is no common root', () => {
    expect(applyZipCommonRootStrip('main.tex', '')).toBe('/main.tex')
  })
})

describe('uniqueExportFolderName', () => {
  it('appends a rootPath basename when names collide', () => {
    const used = new Set<string>()
    const first = uniqueExportFolderName(
      { id: '/docs/Paper-A', rootPath: '/docs/Paper-A', name: 'Paper' } as never,
      used,
    )
    const second = uniqueExportFolderName(
      { id: '/docs/Paper-B', rootPath: '/docs/Paper-B', name: 'Paper' } as never,
      used,
    )
    expect(first).toBe('Paper')
    expect(second).toBe('Paper-Paper-B')
    expect(used.size).toBe(2)
  })
})

describe('normalizeImportEntryPath', () => {
  it('normalizes Windows separators and skips folder markers', () => {
    expect(normalizeImportEntryPath('Alpha\\main.typ')).toBe('Alpha/main.typ')
    expect(normalizeImportEntryPath('Alpha/.folder')).toBeNull()
    expect(normalizeImportEntryPath('__MACOSX/foo.typ')).toBeNull()
  })

  it('resolves in-root .. segments and rejects escapes', () => {
    expect(normalizeImportEntryPath('src/../main.typ')).toBe('main.typ')
    expect(() => normalizeImportEntryPath('../evil.typ')).toThrow(/escapes project root/)
    expect(() => normalizeImportEntryPath('a/../../evil.typ')).toThrow(/escapes project root/)
    expect(() => normalizeImportEntryPath('C:/Users/me/main.typ')).toThrow(/escapes project root/)
  })
})

describe('importLatexProject', () => {
  beforeEach(() => {
    createProjectMock.mockReset()
    createProjectMock.mockResolvedValue('project-id')
  })

  it('throws when project creation is cancelled', async () => {
    createProjectMock.mockResolvedValue('')
    const file = new File(['\\title{Hi}\n\\begin{document}Hi\\end{document}\n'], 'hi.tex', {
      type: 'text/plain',
    })

    await expect(importLatexProject([{ relativePath: 'hi.tex', file }])).rejects.toThrow(
      /cancelled|already exists/i,
    )
  })

  it('passes ifExists fail and select false so imports do not switch UI', async () => {
    const file = new File(['\\begin{document}Hi\\end{document}\n'], 'hi.tex', {
      type: 'text/plain',
    })
    await importLatexProject([{ relativePath: 'hi.tex', file }])
    expect(createProjectMock).toHaveBeenCalledWith(
      expect.any(String),
      expect.objectContaining({
        mainFile: expect.stringMatching(/\.typ$/),
      }),
      { ifExists: 'fail', select: false },
    )
  })

  it('strips a shared top-level folder from relative paths', async () => {
    const main = new File(['\\begin{document}Body\\end{document}\n'], 'main.tex', {
      type: 'text/plain',
    })
    const bib = new File(['@article{a, title={A}}'], 'refs.bib', {
      type: 'text/plain',
    })

    await importLatexProject([
      { relativePath: 'Paper/main.tex', file: main },
      { relativePath: 'Paper/refs.bib', file: bib },
    ])

    const scaffold = createProjectMock.mock.calls[0]?.[1]
    const paths = scaffold.files.map((entry: { path: string }) => entry.path).sort()
    expect(paths).toEqual(['/main.typ', '/refs.bib'])
    expect(scaffold.mainFile).toBe('/main.typ')
  })

  it('names the project from the main file title, not a later chapter author', async () => {
    const main = new File(
      ['\\title{Paper}\\begin{document}Body\\end{document}\n'],
      'main.tex',
      { type: 'text/plain' },
    )
    const chapter = new File(['\\author{Bob}\n'], 'ch2.tex', { type: 'text/plain' })

    const result = await importLatexProject([
      { relativePath: 'main.tex', file: main },
      { relativePath: 'ch2.tex', file: chapter },
    ])

    expect(result.projectName).toBe('Paper')
    expect(createProjectMock.mock.calls[0]?.[0]).toBe('Paper')
    expect(createProjectMock.mock.calls[0]?.[1].mainFile).toBe('/main.typ')
  })

  it('ignores skipped __MACOSX files when naming a one-file import', async () => {
    const junk = new File(['\\begin{document}x\\end{document}\n'], 'x.tex', {
      type: 'text/plain',
    })
    const main = new File(['\\begin{document}Body\\end{document}\n'], 'main.tex', {
      type: 'text/plain',
    })

    await importLatexProject([
      { relativePath: '__MACOSX/x.tex', file: junk },
      { relativePath: 'main.tex', file: main },
    ])

    expect(createProjectMock.mock.calls[0]?.[0]).toBe('main')
  })
})

describe('importLatexZip', () => {
  beforeEach(() => {
    createProjectMock.mockReset()
    createProjectMock.mockResolvedValue('project-id')
  })

  it('strips the shared zip root before creating the project', async () => {
    const zipped = zipSync({
      'Paper/main.tex': strToU8('\\title{Zip Title}\n\\begin{document}Body\\end{document}\n'),
      'Paper/refs.bib': strToU8('@article{a, title={A}}'),
    })
    const file = new File([zipped.buffer.slice(zipped.byteOffset, zipped.byteOffset + zipped.byteLength) as ArrayBuffer], 'paper.zip', { type: 'application/zip' })

    await importLatexZip(file)

    const scaffold = createProjectMock.mock.calls[0]?.[1]
    const paths = scaffold.files.map((entry: { path: string }) => entry.path).sort()
    expect(paths).toEqual(['/main.typ', '/refs.bib'])
    expect(scaffold.mainFile).toBe('/main.typ')
  })

  it('keeps converted .typ when a same-named .typ also ships in the zip', async () => {
    const zipped = zipSync({
      'main.tex': strToU8('\\begin{document}From tex\\end{document}\n'),
      'main.typ': strToU8('= Stale typ\n'),
    })
    const file = new File(
      [zipped.buffer.slice(zipped.byteOffset, zipped.byteOffset + zipped.byteLength) as ArrayBuffer],
      'collision.zip',
      { type: 'application/zip' },
    )

    await importLatexZip(file)

    const scaffold = createProjectMock.mock.calls[0]?.[1]
    const main = scaffold.files.find((entry: { path: string }) => entry.path === '/main.typ')
    expect(scaffold.files).toHaveLength(1)
    expect(main?.content).toContain('// converted')
    expect(main?.content).toContain('From tex')
    expect(main?.content).not.toContain('Stale typ')
  })

  it('unwraps a nested zip when the outer archive has no source files', async () => {
    const inner = zipSync({
      'main.tex': strToU8('\\begin{document}Inner\\end{document}\n'),
    })
    const outer = zipSync({
      'source.zip': inner,
    })
    const file = new File(
      [outer.buffer.slice(outer.byteOffset, outer.byteOffset + outer.byteLength) as ArrayBuffer],
      'nested.zip',
      { type: 'application/zip' },
    )

    await importLatexZip(file)

    const scaffold = createProjectMock.mock.calls[0]?.[1]
    expect(scaffold.files.map((entry: { path: string }) => entry.path)).toEqual(['/main.typ'])
    expect(scaffold.files[0]?.content).toContain('Inner')
  })

  it('converts .ltx files and keeps .cls/.sty as text', async () => {
    const zipped = zipSync({
      'extra.ltx': strToU8('\\begin{document}Ltx\\end{document}\n'),
      'macros.sty': strToU8('\\newcommand{\\foo}{bar}\n'),
      'class.cls': strToU8('\\ProvidesClass{class}\n'),
    })
    const file = new File(
      [zipped.buffer.slice(zipped.byteOffset, zipped.byteOffset + zipped.byteLength) as ArrayBuffer],
      'ltx.zip',
      { type: 'application/zip' },
    )

    await importLatexZip(file)
    const scaffold = createProjectMock.mock.calls[0]?.[1]
    const byPath = Object.fromEntries(
      scaffold.files.map((entry: { path: string; isBinary?: boolean; content?: string }) => [entry.path, entry]),
    )
    expect(Object.keys(byPath).sort()).toEqual(['/class.cls', '/extra.typ', '/macros.sty'])
    expect(byPath['/extra.typ']?.content).toContain('// converted')
    expect(byPath['/macros.sty']?.isBinary).toBe(false)
    expect(byPath['/class.cls']?.isBinary).toBe(false)
  })
})

describe('importAllProjects', () => {
  beforeEach(() => {
    createProjectMock.mockReset()
    createProjectMock.mockResolvedValue('project-id')
  })

  it('converts .tex files to .typ for each project folder', async () => {
    const zipped = zipSync({
      'Alpha/main.tex': strToU8('\\begin{document}Hello\\end{document}\n'),
      'Alpha/notes.typ': strToU8('= Notes\n'),
      'Beta/paper.tex': strToU8('\\begin{document}World\\end{document}\n'),
    })
    const file = new File(
      [zipped.buffer.slice(zipped.byteOffset, zipped.byteOffset + zipped.byteLength) as ArrayBuffer],
      'bundle.zip',
      { type: 'application/zip' },
    )

    const imported = await importAllProjects(file)
    expect(imported).toBe(2)
    expect(createProjectMock).toHaveBeenCalledTimes(2)

    const byName = Object.fromEntries(
      createProjectMock.mock.calls.map((call) => [call[0], call[1]]),
    )

    const alphaPaths = byName.Alpha.files.map((entry: { path: string }) => entry.path).sort()
    expect(alphaPaths).toEqual(['/main.typ', '/notes.typ'])
    expect(byName.Alpha.files.find((entry: { path: string }) => entry.path === '/main.typ')?.content)
      .toContain('// converted')
    expect(byName.Alpha.mainFile).toBe('/main.typ')

    const betaPaths = byName.Beta.files.map((entry: { path: string }) => entry.path)
    expect(betaPaths).toEqual(['/paper.typ'])
    expect(byName.Beta.files[0]?.content).toContain('// converted')
  })

  it('keeps converted .typ on tex+typ path collisions', async () => {
    const zipped = zipSync({
      'Alpha/main.tex': strToU8('\\begin{document}From tex\\end{document}\n'),
      'Alpha/main.typ': strToU8('= Stale typ\n'),
    })
    const file = new File(
      [zipped.buffer.slice(zipped.byteOffset, zipped.byteOffset + zipped.byteLength) as ArrayBuffer],
      'collision.zip',
      { type: 'application/zip' },
    )

    await importAllProjects(file)

    const scaffold = createProjectMock.mock.calls[0]?.[1]
    expect(scaffold.files).toHaveLength(1)
    expect(scaffold.files[0]?.path).toBe('/main.typ')
    expect(scaffold.files[0]?.content).toContain('// converted')
    expect(scaffold.files[0]?.content).toContain('From tex')
    expect(scaffold.files[0]?.content).not.toContain('Stale typ')
    expect(createProjectMock.mock.calls[0]?.[2]).toEqual({ ifExists: 'fail', select: false })
  })

  it('strips a nested shared root inside each project folder', async () => {
    const zipped = zipSync({
      'Paper/src/main.tex': strToU8('\\begin{document}Body\\end{document}\n'),
      'Paper/src/refs.bib': strToU8('@article{a, title={A}}'),
    })
    const file = new File(
      [zipped.buffer.slice(zipped.byteOffset, zipped.byteOffset + zipped.byteLength) as ArrayBuffer],
      'nested.zip',
      { type: 'application/zip' },
    )

    await importAllProjects(file)

    const scaffold = createProjectMock.mock.calls[0]?.[1]
    const paths = scaffold.files.map((entry: { path: string }) => entry.path).sort()
    expect(paths).toEqual(['/main.typ', '/refs.bib'])
    expect(scaffold.mainFile).toBe('/main.typ')
  })

  it('imports projects from Windows-style zip separators', async () => {
    const zipped = zipSync({
      'Alpha\\main.typ': strToU8('= Alpha\n'),
      'Beta\\paper.typ': strToU8('= Beta\n'),
    })
    const file = new File(
      [zipped.buffer.slice(zipped.byteOffset, zipped.byteOffset + zipped.byteLength) as ArrayBuffer],
      'win.zip',
      { type: 'application/zip' },
    )

    const imported = await importAllProjects(file)
    expect(imported).toBe(2)
    const byName = Object.fromEntries(
      createProjectMock.mock.calls.map((call) => [call[0], call[1]]),
    )
    expect(byName.Alpha.files.map((entry: { path: string }) => entry.path)).toEqual(['/main.typ'])
    expect(byName.Beta.files.map((entry: { path: string }) => entry.path)).toEqual(['/paper.typ'])
  })

  it('throws when createProject fails for a project folder', async () => {
    createProjectMock.mockResolvedValue('')
    const zipped = zipSync({
      'Alpha/main.typ': strToU8('= Alpha\n'),
    })
    const file = new File(
      [zipped.buffer.slice(zipped.byteOffset, zipped.byteOffset + zipped.byteLength) as ArrayBuffer],
      'partial.zip',
      { type: 'application/zip' },
    )

    await expect(importAllProjects(file)).rejects.toThrow(/Could not import project "Alpha"/)
  })

  it('keeps earlier successes when a later project is cancelled', async () => {
    createProjectMock
      .mockResolvedValueOnce('/docs/Alpha')
      .mockResolvedValueOnce('')
    const zipped = zipSync({
      'Alpha/main.typ': strToU8('= Alpha\n'),
      'Beta/paper.typ': strToU8('= Beta\n'),
    })
    const file = new File(
      [zipped.buffer.slice(zipped.byteOffset, zipped.byteOffset + zipped.byteLength) as ArrayBuffer],
      'partial.zip',
      { type: 'application/zip' },
    )

    await expect(importAllProjects(file)).resolves.toBe(1)
    expect(createProjectMock).toHaveBeenNthCalledWith(
      2,
      'Beta',
      expect.any(Object),
      expect.objectContaining({ ifExists: 'fail', select: false, parentPath: '/docs' }),
    )
  })

  it('rejects archives with path traversal entries', async () => {
    const zipped = zipSync({
      'Alpha/../../evil.typ': strToU8('= Evil\n'),
    })
    const file = new File(
      [zipped.buffer.slice(zipped.byteOffset, zipped.byteOffset + zipped.byteLength) as ArrayBuffer],
      'slip.zip',
      { type: 'application/zip' },
    )

    await expect(importAllProjects(file)).rejects.toThrow(/escapes project root/)
  })
})

describe('importProject', () => {
  beforeEach(() => {
    createProjectMock.mockReset()
    createProjectMock.mockResolvedValue('project-id')
  })

  it('skips .folder markers and rejects zip-slip paths', async () => {
    const zipped = zipSync({
      'main.typ': strToU8('= Ok\n'),
      '.folder': strToU8(''),
      'chapters/.folder': strToU8(''),
    })
    const file = new File(
      [zipped.buffer.slice(zipped.byteOffset, zipped.byteOffset + zipped.byteLength) as ArrayBuffer],
      'single.zip',
      { type: 'application/zip' },
    )

    await importProject(file)
    const scaffold = createProjectMock.mock.calls[0]?.[1]
    expect(scaffold.files.map((entry: { path: string }) => entry.path)).toEqual(['/main.typ'])

    const slip = zipSync({
      '../evil.typ': strToU8('= Evil\n'),
    })
    const slipFile = new File(
      [slip.buffer.slice(slip.byteOffset, slip.byteOffset + slip.byteLength) as ArrayBuffer],
      'slip.zip',
      { type: 'application/zip' },
    )
    await expect(importProject(slipFile)).rejects.toThrow(/escapes project root/)
  })

  it('imports Overleaf .sty and .cls files as text alongside converted .tex', async () => {
    const zipped = zipSync({
      'Paper/main.tex': strToU8('\\begin{document}Hi\\end{document}\n'),
      'Paper/ieee.cls': strToU8('\\ProvidesClass{ieee}\n'),
      'Paper/macros.sty': strToU8('\\newcommand{\\foo}{bar}\n'),
    })
    const file = new File(
      [zipped.buffer.slice(zipped.byteOffset, zipped.byteOffset + zipped.byteLength) as ArrayBuffer],
      'overleaf.zip',
      { type: 'application/zip' },
    )

    await importProject(file)
    const scaffold = createProjectMock.mock.calls[0]?.[1]
    const byPath = Object.fromEntries(
      scaffold.files.map((entry: { path: string; isBinary?: boolean; content?: string }) => [entry.path, entry]),
    )
    expect(Object.keys(byPath).sort()).toEqual(['/ieee.cls', '/macros.sty', '/main.typ'])
    expect(byPath['/ieee.cls']?.isBinary).toBe(false)
    expect(byPath['/macros.sty']?.content).toContain('\\newcommand')
    expect(byPath['/main.typ']?.content).toContain('// converted')
  })
})

describe('export flush and nesting', () => {
  beforeEach(() => {
    downloadBlobMock.mockReset()
    flushWritesMock.mockReset()
    flushWritesMock.mockResolvedValue({ ok: true })
    getVaultExportBundleMock.mockReset()
    mockProjects = [
      {
        id: '/docs/Solo',
        rootPath: '/docs/Solo',
        name: 'Solo',
        files: [],
        mainFile: 'main.typ',
      },
    ]
  })

  it('throws when flushWrites fails before export', async () => {
    flushWritesMock.mockRejectedValue(new Error('disk busy'))
    getVaultExportBundleMock.mockResolvedValue({
      files: [{ path: '/main.typ', content: '= Hi\n', isBinary: false }],
    })

    await expect(exportProject('/docs/Solo')).rejects.toThrow(/disk busy/)
    expect(getVaultExportBundleMock).not.toHaveBeenCalled()
  })

  it('nests even a single project under its folder for exportAll', async () => {
    getVaultExportBundleMock.mockResolvedValue({
      files: [{ path: '/main.typ', content: '= Solo\n', isBinary: false }],
    })

    await exportAllProjects()

    expect(downloadBlobMock).toHaveBeenCalledTimes(1)
    expect(downloadBlobMock.mock.calls[0]?.[0]).toBe('typsmthng-all-projects.zip')
    const blob = downloadBlobMock.mock.calls[0]?.[1] as Blob
    const buffer = new Uint8Array(await blob.arrayBuffer())
    const unzipped = unzipSync(buffer)
    expect(Object.keys(unzipped).sort()).toEqual(['Solo/main.typ'])
    expect(strFromU8(unzipped['Solo/main.typ']!)).toBe('= Solo\n')
  })

  it('throws when a binary export entry is missing binaryData', async () => {
    getVaultExportBundleMock.mockResolvedValue({
      files: [{ path: 'fig.png', isBinary: true }],
    })

    await expect(exportProject('/docs/Solo')).rejects.toThrow(/Missing binary data/)
  })
})
