import { describe, expect, it } from 'vitest'
import { convertLatexToTypst } from './latex-converter'

describe('convertLatexToTypst', () => {
  describe('bibliography', () => {
    it('appends .bib when the path has no extension', async () => {
      const result = await convertLatexToTypst('\\bibliography{refs}')
      expect(result.typst).toContain('#bibliography("refs.bib")')
      expect(result.typst).not.toContain('refs.bib.bib')
    })

    it('does not double-append .bib when already present', async () => {
      const result = await convertLatexToTypst('\\bibliography{refs.bib}')
      expect(result.typst).toContain('#bibliography("refs.bib")')
      expect(result.typst).not.toContain('refs.bib.bib')
    })

    it('does not double-append .bib for uppercase bibliography paths', async () => {
      const result = await convertLatexToTypst('\\bibliography{refs.BIB}')
      expect(result.typst).toContain('#bibliography("refs.BIB")')
      expect(result.typst).not.toContain('refs.BIB.bib')
    })

    it('normalizes each entry in a comma-separated bibliography list', async () => {
      const result = await convertLatexToTypst('\\bibliography{refs, more.bib, extra}')
      expect(result.typst).toContain(
        '#bibliography(("refs.bib", "more.bib", "extra.bib"))',
      )
    })
  })

  describe('document-less preamble metadata', () => {
    it('extracts title/author/date without \\begin{document}', async () => {
      const source = [
        '\\documentclass{article}',
        '\\usepackage{amsmath}',
        '\\title{Hello World}',
        '\\author{Ada Lovelace}',
        '\\date{2024}',
        '',
        'Body text here.',
      ].join('\n')

      const result = await convertLatexToTypst(source)

      expect(result.metadata.title).toBe('Hello World')
      expect(result.metadata.author).toBe('Ada Lovelace')
      expect(result.metadata.date).toBe('2024')
      expect(result.metadata.documentclass).toBe('article')
      expect(result.metadata.packages).toContain('amsmath')

      expect(result.typst).toContain('#set document(')
      expect(result.typst).toContain('title: [Hello World],')
      expect(result.typst).toContain('author: "Ada Lovelace",')
      expect(result.typst).toContain('date: datetime(year: 2024, month: 1, day: 1),')
      expect(result.typst).not.toContain('date: "')
      expect(result.typst).toContain('Body text here.')

      // Consumed preamble macros should not be re-emitted as body noise
      expect(result.typst).not.toContain('\\title')
      expect(result.typst).not.toContain('\\author')
      expect(result.typst).not.toMatch(/Unsupported: \\title/)
      expect(result.typst).not.toMatch(/Unsupported: \\author/)
      expect(result.typst).not.toMatch(/Unsupported: \\documentclass/)
      expect(result.typst).not.toMatch(/Unsupported: \\usepackage/)
    })

    it('still extracts metadata when \\begin{document} is present', async () => {
      const source = [
        '\\documentclass{article}',
        '\\title{With Doc}',
        '\\author{Test Author}',
        '\\begin{document}',
        'Hello.',
        '\\end{document}',
      ].join('\n')

      const result = await convertLatexToTypst(source)
      expect(result.metadata.title).toBe('With Doc')
      expect(result.metadata.author).toBe('Test Author')
      expect(result.typst).toContain('Hello.')
      expect(result.typst).toContain('#set document(')
    })
  })

  describe('maketitle', () => {
    it('emits a Typst title block instead of an empty string', async () => {
      const source = [
        '\\title{My Paper}',
        '\\author{Someone}',
        '\\begin{document}',
        '\\maketitle',
        'Intro.',
        '\\end{document}',
      ].join('\n')

      const result = await convertLatexToTypst(source)

      expect(result.typst).toContain('#align(center)[')
      expect(result.typst).toContain('#document.title')
      expect(result.typst).toContain('#document.author')
      expect(result.typst).toContain('Intro.')
      // Should not leave a blank gap where maketitle used to emit ""
      expect(result.typst.trim()).not.toBe('#set document(\n  title: [My Paper],\n  author: "Someone",\n)\n\nIntro.')
    })
  })

  describe('basic conversions', () => {
    it('converts section and textbf', async () => {
      const source = [
        '\\begin{document}',
        '\\section{Introduction}',
        'This is \\textbf{bold} text.',
        '\\end{document}',
      ].join('\n')

      const result = await convertLatexToTypst(source)

      expect(result.typst).toContain('= Introduction')
      expect(result.typst).toContain('*bold*')
    })

    it('converts subsections and textit/emph', async () => {
      const source = '\\subsection{Details}\n\\textit{italic} and \\emph{also}'
      const result = await convertLatexToTypst(source)

      expect(result.typst).toContain('== Details')
      expect(result.typst).toContain('_italic_')
      expect(result.typst).toContain('_also_')
    })
  })

  describe('cite and ref', () => {
    it('converts \\cite and \\ref to Typst references', async () => {
      const source = 'See \\cite{knuth84} and Figure~\\ref{fig:plot}.'
      const result = await convertLatexToTypst(source)

      expect(result.typst).toContain('@knuth84')
      expect(result.typst).toContain('@fig:plot')
    })

    it('splits multi-key cite commands', async () => {
      const result = await convertLatexToTypst('See \\cite{knuth84,lamport94}.')
      expect(result.typst).toContain('@knuth84 @lamport94')
    })

    it('converts \\eqref and cite variants', async () => {
      const source = '\\eqref{eq:1} \\citep{a} \\citet{b}'
      const result = await convertLatexToTypst(source)

      expect(result.typst).toContain('@eq:1')
      expect(result.typst).toContain('@a')
      expect(result.typst).toContain('@b')
    })
  })

  describe('input and include path rewriting', () => {
    it('rewrites .tex includes to .typ', async () => {
      const result = await convertLatexToTypst('\\input{chapters/intro.tex}')
      expect(result.typst).toContain('#include "chapters/intro.typ"')
    })

    it('appends .typ when no extension is given', async () => {
      const result = await convertLatexToTypst('\\include{methods}')
      expect(result.typst).toContain('#include "methods.typ"')
    })

  it('rewrites .TEX includes case-insensitively', async () => {
    const result = await convertLatexToTypst('\\input{chapters/INTRO.TEX}')
    expect(result.typst).toContain('#include "chapters/INTRO.typ"')
    expect(result.typst).not.toContain('.TEX.typ')
  })
  })

  describe('table environment with nested tabular', () => {
    it('emits the table body for \\begin{table}...\\begin{tabular}', async () => {
      const source =
        '\\begin{table}\\begin{tabular}{cc} a & b \\\\ \\end{tabular}\\end{table}'
      const result = await convertLatexToTypst(source)

      expect(result.typst).toContain('#table(')
      expect(result.typst).toContain('columns: 2')
      expect(result.typst).toContain('[a], [b]')
    })

    it('emits the table body for the table* variant', async () => {
      const source =
        '\\begin{table*}\\begin{tabular}{cc} a & b \\\\ \\end{tabular}\\end{table*}'
      const result = await convertLatexToTypst(source)

      expect(result.typst).toContain('#table(')
      expect(result.typst).toContain('columns: 2')
      expect(result.typst).toContain('[a], [b]')
    })

    it('counts only column letters when colspec includes vertical rules', async () => {
      const source =
        '\\begin{tabular}{|l|c|r|} a & b & c \\\\ \\end{tabular}'
      const result = await convertLatexToTypst(source)

      expect(result.typst).toContain('columns: 3')
      expect(result.typst).toContain('[a], [b], [c]')
    })

    it('does not count letters inside p{width} as columns', async () => {
      const source =
        '\\begin{tabular}{p{2cm}c} a & b \\\\ \\end{tabular}'
      const result = await convertLatexToTypst(source)

      expect(result.typst).toContain('columns: 2')
      expect(result.typst).toContain('[a], [b]')
    })

    it('expands *{n}{inner} column repetition', async () => {
      const source =
        '\\begin{tabular}{*{3}{c}} a & b & c \\\\ \\end{tabular}'
      const result = await convertLatexToTypst(source)

      expect(result.typst).toContain('columns: 3')
      expect(result.typst).toContain('[a], [b], [c]')
    })

    it('clamps huge *{n}{inner} repeat counts instead of allocating them', async () => {
      const source =
        '\\begin{tabular}{*{2000000000}{c}} a & b \\\\ \\end{tabular}'
      const result = await convertLatexToTypst(source)

      expect(result.typst).toContain('columns: 100')
      expect(result.typst).toContain('[a], [b]')
    })

    it('ignores >{...} decorators when counting columns', async () => {
      const source =
        '\\begin{tabular}{>{\\bfseries}lcr} a & b & c \\\\ \\end{tabular}'
      const result = await convertLatexToTypst(source)

      expect(result.typst).toContain('columns: 3')
      expect(result.typst).toContain('[a], [b], [c]')
    })
  })

  describe('math line breaks', () => {
    it('converts align \\\\ without hanging', async () => {
      const result = await convertLatexToTypst(
        '\\begin{align} a &= b \\\\ c &= d \\end{align}',
      )
      expect(result.typst).toContain('a')
      expect(result.typst).toContain('b')
      expect(result.typst).toContain('c')
      expect(result.typst).toContain('d')
    })

    it('converts escaped ampersand in inline math', async () => {
      const result = await convertLatexToTypst('$ a \\& b $')
      expect(result.typst).toContain('a')
      expect(result.typst).toContain('&')
      expect(result.typst).toContain('b')
    })
  })

  describe('typst validity', () => {
    it('emits date: auto for \\today', async () => {
      const result = await convertLatexToTypst(
        '\\title{Hi}\\date{\\today}\\begin{document}x\\end{document}',
      )
      expect(result.typst).toContain('date: auto,')
      expect(result.typst).not.toContain('date: "')
    })

    it('always gives #figure a positional body', async () => {
      const result = await convertLatexToTypst(
        '\\begin{figure}\\caption{A fig}\\label{fig:a}\\end{figure}',
      )
      expect(result.typst).toContain('#figure(')
      expect(result.typst).toMatch(/#figure\(\s*\[\/\* figure content missing \*\/\],/)
      expect(result.typst).toContain('caption: [A fig]')
    })

    it('escapes brackets and quotes in interpolated Typst syntax', async () => {
      const titled = await convertLatexToTypst(
        '\\title{The ] bracket}\\author{John "Jack" Smith}\\begin{document}x\\end{document}',
      )
      expect(titled.typst).toContain('title: [The \\] bracket]')
      expect(titled.typst).toContain('author: "John \\"Jack\\" Smith"')

      const linked = await convertLatexToTypst('\\href{https://ex.com/a"b}{click}')
      expect(linked.typst).toContain('#link("https://ex.com/a\\"b")[click]')
    })
  })

  describe('adversarial input', () => {
    it('drops content nested beyond the emit depth limit instead of overflowing the stack', async () => {
      const depth = 2000
      const source = '{'.repeat(depth) + 'deep' + '}'.repeat(depth)
      const result = await convertLatexToTypst(source)
      expect(result.warnings.some((warning) => warning.construct === 'deep-nesting')).toBe(true)

      // Shallow nesting is unaffected.
      const shallow = await convertLatexToTypst('{{{ok}}}')
      expect(shallow.typst).toContain('ok')
      expect(shallow.warnings.some((warning) => warning.construct === 'deep-nesting')).toBe(false)
    })
  })
})
