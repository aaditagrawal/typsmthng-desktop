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
      expect(result.typst).toContain('date: "2024",')
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

    it('ignores >{...} decorators when counting columns', async () => {
      const source =
        '\\begin{tabular}{>{\\bfseries}lcr} a & b & c \\\\ \\end{tabular}'
      const result = await convertLatexToTypst(source)

      expect(result.typst).toContain('columns: 3')
      expect(result.typst).toContain('[a], [b], [c]')
    })
  })
})
