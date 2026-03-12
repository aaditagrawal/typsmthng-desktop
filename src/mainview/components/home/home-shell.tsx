import { useState } from 'react'
import { ProjectPicker } from '@/components/home/project-picker'
import { GuidePage } from '@/components/home/guide-page'

interface HomeShellProps {
  onPreloadWorkspace?: () => void
}

export default function HomeShell({ onPreloadWorkspace }: HomeShellProps) {
  const [showGuide, setShowGuide] = useState(false)

  if (showGuide) {
    return (
      <div
        className="h-full w-full relative"
        style={{ background: 'var(--bg-app)' }}
      >
        <div className="electrobun-webkit-app-region-drag absolute top-0 left-0 right-0 h-10" />
        <GuidePage onBack={() => setShowGuide(false)} />
      </div>
    )
  }

  return (
    <div
      className="h-full w-full relative"
      style={{ background: 'var(--bg-app)' }}
    >
      <div className="electrobun-webkit-app-region-drag absolute top-0 left-0 right-0 h-10" />
      <ProjectPicker
        onShowGuide={() => setShowGuide(true)}
        onPreloadWorkspace={onPreloadWorkspace}
      />
    </div>
  )
}
