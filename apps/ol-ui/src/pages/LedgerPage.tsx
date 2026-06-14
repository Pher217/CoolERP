import { useCallback, useRef } from 'react'
import { AccountsPanel } from '../components/AccountsPanel.tsx'
import { PostEntryForm } from '../components/PostEntryForm.tsx'

export function LedgerPage() {
  const refreshBalancesRef = useRef<() => void>(() => {})

  const registerRefresh = useCallback((fn: () => void) => {
    refreshBalancesRef.current = fn
  }, [])

  const handlePostSuccess = useCallback(() => {
    refreshBalancesRef.current()
  }, [])

  return (
    <div className="mx-auto max-w-6xl space-y-6">
      <h1 className="text-2xl font-bold tracking-tight text-foreground">Journal Entries</h1>
      <AccountsPanel onRegisterRefresh={registerRefresh} />
      <PostEntryForm onSuccess={handlePostSuccess} />
    </div>
  )
}
