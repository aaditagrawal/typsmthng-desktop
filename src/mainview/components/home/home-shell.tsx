import { useState } from 'react'
import { ProjectPicker } from '@/components/home/project-picker'
import { GuidePage } from '@/components/home/guide-page'
import { UpdateBanner } from '@/components/layout/update-banner'
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

  return (
    <div
      className="h-full w-full relative"
      style={{ background: 'var(--bg-app)' }}
    >
      <TitleDragRegion />
      {/* Below the titleband so update actions stay clickable despite the drag strip. */}
      <div className="absolute left-0 right-0 z-20" style={{ top: '40px' }}>
        <UpdateBanner />
      </div>
      {showGuide ? (
        <GuidePage onBack={() => setShowGuide(false)} />
      ) : (
        <ProjectPicker
          onShowGuide={() => setShowGuide(true)}
          onPreloadWorkspace={onPreloadWorkspace}
        />
      )}
    </div>
  )
}
