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
    <div className="space-y-6">
      <AccountsPanel onRegisterRefresh={registerRefresh} />
      <PostEntryForm onSuccess={handlePostSuccess} />
    </div>
  )
}
