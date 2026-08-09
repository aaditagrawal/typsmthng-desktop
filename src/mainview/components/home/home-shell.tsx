import { useState } from 'react'
import { ProjectPicker } from '@/components/home/project-picker'
import { GuidePage } from '@/components/home/guide-page'
import { isMacOS } from '@/lib/platform'

interface HomeShellProps {
  onPreloadWorkspace?: () => void
}

function TitleDragRegion() {
  // Overlay only the empty titleband (home/guide content pads below h-10).
  // Inset on macOS so the strip does not cover traffic-light hit targets.
  // Keep this above the scroll surface (default paint order) so Linux/Windows
  // can still drag; interactive controls live below the strip via padding.
  return (
    <div
      className="electrobun-webkit-app-region-drag absolute top-0 right-0 h-10"
      style={{ left: isMacOS ? '78px' : 0 }}
      aria-hidden
    />
  )
}

export default function HomeShell({ onPreloadWorkspace }: HomeShellProps) {
  const [showGuide, setShowGuide] = useState(false)

  if (showGuide) {
    return (
      <div
        className="h-full w-full relative"
        style={{ background: 'var(--bg-app)' }}
      >
        <TitleDragRegion />
        <GuidePage onBack={() => setShowGuide(false)} />
      </div>
    )
  }

  return (
    <div
      className="h-full w-full relative"
      style={{ background: 'var(--bg-app)' }}
    >
      <TitleDragRegion />
      <ProjectPicker
        onShowGuide={() => setShowGuide(true)}
        onPreloadWorkspace={onPreloadWorkspace}
      />
    </div>
  )
}
