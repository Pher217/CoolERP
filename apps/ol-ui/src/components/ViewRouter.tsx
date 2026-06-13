import { AccountsPanel } from './AccountsPanel.tsx'
import { PostEntryForm } from './PostEntryForm.tsx'
import { InventoryPanel } from './InventoryPanel.tsx'
import { WorkflowView } from './WorkflowView.tsx'

export type View = 'ledger' | 'inventory' | 'workflows'

type Props = {
  activeView: View
  onChangeView: (view: View) => void
  onRegisterRefresh: (fn: () => void) => void
  onPostSuccess: () => void
}

const TABS: { id: View; label: string }[] = [
  { id: 'ledger', label: 'Ledger' },
  { id: 'inventory', label: 'Inventory' },
  { id: 'workflows', label: 'Workflows' },
]

export function ViewRouter({
  activeView,
  onChangeView,
  onRegisterRefresh,
  onPostSuccess,
}: Props) {
  return (
    <div className="view-router">
      <div className="view-tabs" role="tablist" aria-label="ERP views">
        {TABS.map((tab) => (
          <button
            key={tab.id}
            className={`view-tab ${activeView === tab.id ? 'active' : ''}`}
            onClick={() => onChangeView(tab.id)}
            role="tab"
            aria-selected={activeView === tab.id}
          >
            {tab.label}
          </button>
        ))}
      </div>
      <div className="workspace-content">
        {activeView === 'ledger' && (
          <>
            <AccountsPanel onRegisterRefresh={onRegisterRefresh} />
            <PostEntryForm onSuccess={onPostSuccess} />
          </>
        )}
        {activeView === 'inventory' && <InventoryPanel />}
        {activeView === 'workflows' && <WorkflowView />}
      </div>
    </div>
  )
}
