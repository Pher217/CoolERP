import { BrowserRouter, Routes, Route } from 'react-router-dom'
import { AppShell } from './components/AppShell.tsx'
import { Overview } from './pages/Overview.tsx'
import { LedgerPage } from './pages/LedgerPage.tsx'
import { InventoryPanel } from './components/InventoryPanel.tsx'
import { WorkflowView } from './components/WorkflowView.tsx'

export default function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route element={<AppShell />}>
          <Route index element={<Overview />} />
          <Route path="ledger" element={<LedgerPage />} />
          <Route path="inventory" element={<InventoryPanel />} />
          <Route path="processes" element={<WorkflowView />} />
        </Route>
      </Routes>
    </BrowserRouter>
  )
}
