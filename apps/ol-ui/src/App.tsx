import { useCallback, useRef, useState } from 'react'
import { ChatPanel } from './components/ChatPanel.tsx'
import { ViewRouter, type View } from './components/ViewRouter.tsx'
import './App.css'

export default function App() {
  const [activeView, setActiveView] = useState<View>('ledger')

  // accountsPanelRef allows PostEntryForm to trigger a balances refresh
  const refreshBalancesRef = useRef<() => void>(() => {})

  const registerRefresh = useCallback((fn: () => void) => {
    refreshBalancesRef.current = fn
  }, [])

  const handlePostSuccess = useCallback(() => {
    refreshBalancesRef.current()
  }, [])

  const handleView = useCallback((module: string) => {
    if (module === 'ledger' || module === 'inventory' || module === 'workflows') {
      setActiveView(module)
    }
  }, [])

  return (
    <div className="app">
      <header className="app-header">
        <span className="logo">OpenERP</span>
        <span className="tagline">Correct by construction · Agent-native by design</span>
      </header>
      <main className="workspace">
        <div className="workspace-left">
          <ViewRouter
            activeView={activeView}
            onChangeView={setActiveView}
            onRegisterRefresh={registerRefresh}
            onPostSuccess={handlePostSuccess}
          />
        </div>
        <div className="workspace-right">
          <ChatPanel onView={handleView} />
        </div>
      </main>
    </div>
  )
}
