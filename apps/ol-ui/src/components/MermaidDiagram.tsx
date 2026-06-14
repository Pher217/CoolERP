import { useEffect, useRef } from 'react'
import mermaid from 'mermaid'

mermaid.initialize({ startOnLoad: false, theme: 'neutral' })

let mermaidIdCounter = 0

export function MermaidDiagram({ chart }: { chart: string }) {
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!ref.current) return
    const id = `mermaid-${++mermaidIdCounter}`
    mermaid
      .render(id, chart)
      .then(({ svg }) => {
        if (ref.current) ref.current.innerHTML = svg
      })
      .catch(() => {
        if (ref.current) ref.current.textContent = 'Diagram render failed.'
      })
  }, [chart])

  return <div ref={ref} className="overflow-x-auto rounded-lg border bg-muted/30 p-4" />
}
