import { describe, expect, it, vi, beforeEach } from 'vitest'
import { zipSync, strToU8 } from 'fflate'

const createProjectMock = vi.fn()

vi.mock('@/stores/project-store', () => ({
  useProjectStore: Object.assign(
    vi.fn(),
    {
      getState: () => ({
        createProject: createProjectMock,
        getCurrentProject: () => null,
        projects: [],
      }),
      setState: vi.fn(),
    },
  ),
}))

vi.mock('@/lib/latex-converter', () => ({
  convertLatexToTypst: vi.fn(async (source: string) => ({
    typst: `// converted\n${source}`,
    warnings: [],
    metadata: { packages: [], title: 'Converted Doc' },
  })),
}))

vi.mock('@/lib/desktop-rpc', () => ({
  desktopRpc: {
    request: {
      getVaultExportBundle: vi.fn(),
    },
  },
}))

import {
  applyZipCommonRootStrip,
  importAllProjects,
  importLatexProject,
  importLatexZip,
  resolveImportedMainFile,
  stripZipCommonRootPrefix,
} from './project-io'

describe('resolveImportedMainFile', () => {
  it('prefers /main.typ when present', () => {
    expect(resolveImportedMainFile([
      { path: '/chapters/intro.typ' },
      { path: '/main.typ' },
    ])).toBe('/main.typ')
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

  it('passes ifExists fail so collisions are not silent successes', async () => {
    const file = new File(['\\begin{document}Hi\\end{document}\n'], 'hi.tex', {
      type: 'text/plain',
    })
    await importLatexProject([{ relativePath: 'hi.tex', file }])
    expect(createProjectMock).toHaveBeenCalledWith(
      expect.any(String),
      expect.objectContaining({
        mainFile: expect.stringMatching(/\.typ$/),
      }),
      { ifExists: 'fail' },
    )
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
})
