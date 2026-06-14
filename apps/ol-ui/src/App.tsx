import { BrowserRouter, Routes, Route } from 'react-router-dom'
import { Toaster } from '@/components/ui/sonner'
import { AppShell } from './components/AppShell.tsx'
import { Overview } from './pages/Overview.tsx'
import { LedgerPage } from './pages/LedgerPage.tsx'
import { Accounts } from './pages/Accounts.tsx'
import { Receivables } from './pages/Receivables.tsx'
import { Payables } from './pages/Payables.tsx'
import { InventoryPanel } from './components/InventoryPanel.tsx'
import { WorkflowView } from './components/WorkflowView.tsx'
import { Settings } from './pages/Settings.tsx'

export default function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route element={<AppShell />}>
          <Route index element={<Overview />} />
          <Route path="ledger" element={<LedgerPage />} />
          <Route path="accounts" element={<Accounts />} />
          <Route path="receivables" element={<Receivables />} />
          <Route path="payables" element={<Payables />} />
          <Route path="inventory" element={<InventoryPanel />} />
          <Route path="processes" element={<WorkflowView />} />
          <Route path="settings" element={<Settings />} />
        </Route>
      </Routes>
      <Toaster position="bottom-right" />
    </BrowserRouter>
  )
}
