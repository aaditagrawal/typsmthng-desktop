import { useEffect, useMemo, useRef, useState } from 'react'
import { FileText, Search, TextCursorInput } from 'lucide-react'
import { useShallow } from 'zustand/react/shallow'

import { desktopRpc } from '@/lib/desktop-rpc'
import { useProjectStore } from '@/stores/project-store'
import { useUIStore } from '@/stores/ui-store'

type SearchItem =
  | {
      kind: 'path'
      id: string
      title: string
      subtitle: string
      path: string
    }
  | {
      kind: 'text'
      id: string
      title: string
      subtitle: string
      path: string
      line: number
    }

const PATH_RESULT_LIMIT = 18
const TEXT_RESULT_LIMIT = 18

export function CommandSearch() {
  const open = useUIStore((s) => s.commandSearchOpen)
  const setOpen = useUIStore((s) => s.setCommandSearchOpen)
  const { currentProjectId, currentProjectName, includeHidden } = useProjectStore(
    useShallow((s) => {
      const currentProject = s.metadata?.recentVaults.find((vault) => vault.rootPath === s.currentProjectId) ?? null
      return {
        currentProjectId: s.currentProjectId,
        currentProjectName: currentProject?.name ?? null,
        includeHidden: currentProject?.hiddenFilesVisible ?? false,
      }
    }),
  )

  const [query, setQuery] = useState('')
  const [pathItems, setPathItems] = useState<SearchItem[]>([])
  const [textItems, setTextItems] = useState<SearchItem[]>([])
  const [selectedIndex, setSelectedIndex] = useState(0)
  const [searching, setSearching] = useState(false)

  const inputRef = useRef<HTMLInputElement>(null)

  const items = useMemo(() => [...pathItems, ...textItems], [pathItems, textItems])

  useEffect(() => {
    if (!open) return
    requestAnimationFrame(() => inputRef.current?.focus())
  }, [open])

  useEffect(() => {
    if (!open || !currentProjectId) return

    const trimmed = query.trim()
    if (!trimmed) {
      setPathItems([])
      setTextItems([])
      setSearching(false)
      setSelectedIndex(0)
      return
    }

    let cancelled = false
    setSearching(true)
    const timer = setTimeout(async () => {
      try {
        const [pathResults, textResults] = await Promise.all([
          desktopRpc.request.searchVaultPaths({
            rootPath: currentProjectId,
            query: trimmed,
            limit: PATH_RESULT_LIMIT,
            includeHidden,
          }),
          trimmed.length < 2
            ? Promise.resolve({ results: [], truncated: false })
            : desktopRpc.request.searchVaultText({
                rootPath: currentProjectId,
                query: trimmed,
                limit: TEXT_RESULT_LIMIT,
                includeHidden,
              }),
        ])

        if (cancelled) return

        setPathItems(
          pathResults.results.map((result) => ({
            kind: 'path' as const,
            id: `path:${result.path}`,
            title: result.name,
            subtitle: result.path,
            path: result.path,
          })),
        )
        setTextItems(
          textResults.results.map((result) => ({
            kind: 'text' as const,
            id: `text:${result.path}:${result.line}:${result.column}`,
            title: `${result.path}:${result.line}`,
            subtitle: result.preview,
            path: result.path,
            line: result.line,
          })),
        )
        setSelectedIndex(0)
      } finally {
        if (!cancelled) setSearching(false)
      }
    }, 120)

    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [currentProjectId, includeHidden, open, query])

  const close = () => {
    setOpen(false)
    setQuery('')
    setSelectedIndex(0)
    setPathItems([])
    setTextItems([])
  }

  const openItem = async (item: SearchItem | undefined) => {
    if (!item) return
    useProjectStore.getState().selectFile(item.path)
    useProjectStore.getState().setSidebarOpen(true)
    close()
  }

  if (!open) return null

  const selected = items[Math.min(selectedIndex, Math.max(items.length - 1, 0))]

  return (
    <div
      className="fixed inset-0 z-[120] flex items-start justify-center px-5 pt-20"
      style={{ background: 'rgba(0, 0, 0, 0.6)' }}
      onClick={(event) => {
        if (event.target === event.currentTarget) close()
      }}
    >
      <div
        className="w-full max-w-[760px] overflow-hidden"
        style={{
          background: 'rgba(20, 20, 20, 0.45)',
          backdropFilter: 'blur(50px) saturate(180%)',
          WebkitBackdropFilter: 'blur(50px) saturate(180%)',
          border: '1px solid var(--border-strong)',
          borderRadius: '2px',
          boxShadow: '0 24px 48px rgba(0, 0, 0, 0.28)',
        }}
        onKeyDown={(event) => {
          if (event.key === 'Escape') {
            event.preventDefault()
            close()
          } else if (event.key === 'ArrowDown') {
            event.preventDefault()
            setSelectedIndex((current) => Math.min(current + 1, Math.max(items.length - 1, 0)))
          } else if (event.key === 'ArrowUp') {
            event.preventDefault()
            setSelectedIndex((current) => Math.max(current - 1, 0))
          } else if (event.key === 'Enter') {
            event.preventDefault()
            void openItem(selected)
          }
        }}
      >
        <div className="flex items-center gap-3 px-4 py-3" style={{ borderBottom: '1px solid var(--border-default)' }}>
          <Search size={16} style={{ color: 'var(--text-tertiary)' }} />
          <input
            ref={inputRef}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="SEARCH FILES AND TEXT..."
            style={{
              flex: 1,
              background: 'transparent',
              border: 'none',
              outline: 'none',
              color: 'var(--text-primary)',
              fontFamily: 'var(--font-mono)',
              fontSize: '14px',
              letterSpacing: '0.04em',
            }}
          />
          <div style={{ color: 'var(--text-tertiary)', fontFamily: 'var(--font-mono)', fontSize: '11px', letterSpacing: '0.08em', textTransform: 'uppercase' }}>
            {searching ? 'searching' : currentProjectName ?? 'no project'}
          </div>
        </div>

        <div className="max-h-[65vh] overflow-y-auto px-2 py-2">
          {items.length === 0 ? (
            <div
              className="px-4 py-10 text-center"
              style={{
                color: 'var(--text-secondary)',
                fontFamily: 'var(--font-mono)',
                fontSize: '12px',
                letterSpacing: '0.04em',
                textTransform: 'uppercase',
              }}
            >
              {query.trim() ? 'No matches yet.' : 'Start typing to search the active project.'}
            </div>
          ) : (
            <>
              {pathItems.length > 0 && (
                <div className="mb-4">
                  <div style={{ color: 'var(--text-tertiary)', fontFamily: 'var(--font-mono)', fontSize: '11px', letterSpacing: '0.1em', textTransform: 'uppercase', padding: '0 12px 8px' }}>
                    Files
                  </div>
                  {pathItems.map((item) => {
                    const index = items.findIndex((candidate) => candidate.id === item.id)
                    const isSelected = selectedIndex === index
                    return (
                      <button
                        key={item.id}
                        type="button"
                        onClick={() => void openItem(item)}
                        onMouseEnter={() => setSelectedIndex(index)}
                        className="flex w-full items-start gap-3 px-3 py-3 text-left transition"
                        style={{
                          background: isSelected ? 'var(--accent-muted)' : 'transparent',
                          borderLeft: isSelected ? '2px solid var(--accent)' : '2px solid transparent',
                        }}
                      >
                        <FileText size={15} style={{ color: isSelected ? 'var(--accent)' : 'var(--text-tertiary)', marginTop: '2px' }} />
                        <div className="min-w-0 flex-1">
                          <div className="truncate" style={{ color: 'var(--text-primary)', fontWeight: 600 }}>
                            {item.title}
                          </div>
                          <div className="truncate" style={{ color: 'var(--text-secondary)', fontSize: '12px', marginTop: '4px' }}>
                            {item.subtitle}
                          </div>
                        </div>
                      </button>
                    )
                  })}
                </div>
              )}

              {textItems.length > 0 && (
                <div>
                  <div style={{ color: 'var(--text-tertiary)', fontFamily: 'var(--font-mono)', fontSize: '11px', letterSpacing: '0.1em', textTransform: 'uppercase', padding: '0 12px 8px' }}>
                    Text Matches
                  </div>
                  {textItems.map((item) => {
                    const index = items.findIndex((candidate) => candidate.id === item.id)
                    const isSelected = selectedIndex === index
                    return (
                      <button
                        key={item.id}
                        type="button"
                        onClick={() => void openItem(item)}
                        onMouseEnter={() => setSelectedIndex(index)}
                        className="flex w-full items-start gap-3 px-3 py-3 text-left transition"
                        style={{
                          background: isSelected ? 'var(--accent-muted)' : 'transparent',
                          borderLeft: isSelected ? '2px solid var(--accent)' : '2px solid transparent',
                        }}
                      >
                        <TextCursorInput size={15} style={{ color: isSelected ? 'var(--accent)' : 'var(--text-tertiary)', marginTop: '2px' }} />
                        <div className="min-w-0 flex-1">
                          <div className="truncate" style={{ color: 'var(--text-primary)', fontWeight: 600 }}>
                            {item.title}
                          </div>
                          <div style={{ color: 'var(--text-secondary)', fontSize: '12px', marginTop: '4px' }}>
                            {item.subtitle}
                          </div>
                        </div>
                      </button>
                    )
                  })}
                </div>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  )
}
