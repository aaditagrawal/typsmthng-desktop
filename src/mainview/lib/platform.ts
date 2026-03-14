export const isLinux = /Linux/.test(navigator.userAgent)
export const isMacOS = /Mac/.test(navigator.userAgent)
export const isWindows = /Windows/.test(navigator.userAgent)

export const fileManagerName = isMacOS ? 'Finder' : isWindows ? 'Explorer' : 'Files'
export const revealLabel = `Reveal in ${fileManagerName}`
