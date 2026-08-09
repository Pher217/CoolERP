import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { render, screen, waitFor, cleanup } from '@testing-library/react'
import { MemoryRouter, Routes, Route } from 'react-router-dom'

import { ProcessOverview } from './ProcessOverview.tsx'
import { api } from '../api/client.ts'
import { humanize } from '../lib/humanize.ts'

vi.mock('../api/client.ts', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/client.ts')>()
  return { ...actual, api: { ...actual.api, getProcess: vi.fn() } }
})

vi.mock('../components/MermaidDiagram.tsx', () => ({
  MermaidDiagram: ({ chart }: { chart: string }) => <div data-testid="mermaid">{chart}</div>,
}))

describe('ProcessOverview', () => {
  beforeEach(() => {
    vi.resetAllMocks()
  })

  afterEach(() => {
    cleanup()
  })

  it('shows the state machine diagram and transitions for a 0-step process', async () => {
    /**
     * GIVEN a process with 0 steps but 3 transitions and a non-empty mermaid string
     * WHEN the page loads
     * THEN the "State machine" heading is in the document AND the mocked mermaid element
     * has the exact mermaid text AND every transition's humanized `from` label appears.
     */
    const process = {
      name: 'customer_invoice',
      process: 'customer_invoice',
      states: ['draft', 'sent', 'paid'],
      transitions: [
        { from: 'draft', to: 'sent', capability: 'send_invoice' },
        { from: 'sent', to: 'paid' },
        { from: 'paid', to: 'reconciled' },
      ],
      mermaid: 'graph LR\n  draft --> sent\n  sent --> paid\n  paid --> reconciled',
      steps: [],
    }
    ;(api.getProcess as ReturnType<typeof vi.fn>).mockResolvedValue(process)

    render(
      <MemoryRouter initialEntries={['/processes/customer_invoice']}>
        <Routes>
          <Route path="/processes/:name" element={<ProcessOverview />} />
        </Routes>
      </MemoryRouter>,
    )

    await waitFor(() => {
      expect(api.getProcess).toHaveBeenCalledWith('customer_invoice')
    })

    expect(screen.getByText('State machine')).toBeInTheDocument()
    expect(screen.getByTestId('mermaid').textContent).toBe(process.mermaid)

    screen.getByRole('tab', { name: 'Transitions' }).click()
    await waitFor(() => {
      expect(screen.getByRole('columnheader', { name: 'From' })).toBeInTheDocument()
    })

    for (const transition of process.transitions) {
      expect(screen.getAllByText(humanize(transition.from)).length).toBeGreaterThan(0)
    }
  })

  it('keeps the "No steps defined" notice for a 0-step process', async () => {
    /**
     * GIVEN the same 0-step process
     * WHEN the page loads
     * THEN "No steps defined" is still shown (the empty-steps notice is not removed).
     */
    const process = {
      name: 'customer_invoice',
      process: 'customer_invoice',
      states: ['draft', 'sent', 'paid'],
      transitions: [
        { from: 'draft', to: 'sent', capability: 'send_invoice' },
        { from: 'sent', to: 'paid' },
        { from: 'paid', to: 'reconciled' },
      ],
      mermaid: 'graph LR\n  draft --> sent\n  sent --> paid\n  paid --> reconciled',
      steps: [],
    }
    ;(api.getProcess as ReturnType<typeof vi.fn>).mockResolvedValue(process)

    render(
      <MemoryRouter initialEntries={['/processes/customer_invoice']}>
        <Routes>
          <Route path="/processes/:name" element={<ProcessOverview />} />
        </Routes>
      </MemoryRouter>,
    )

    await waitFor(() => {
      expect(screen.getByText(/no steps defined/i)).toBeInTheDocument()
    })
  })

  it('hides the state machine card when there are no transitions and no mermaid diagram', async () => {
    /**
     * GIVEN a process with 0 steps, 0 transitions and an empty mermaid string
     * WHEN the page loads
     * THEN "No steps defined" is shown AND the "State machine" heading is NOT in the document.
     */
    const process = {
      name: 'empty_process',
      process: 'empty_process',
      states: [],
      transitions: [],
      mermaid: '',
      steps: [],
    }
    ;(api.getProcess as ReturnType<typeof vi.fn>).mockResolvedValue(process)

    render(
      <MemoryRouter initialEntries={['/processes/empty_process']}>
        <Routes>
          <Route path="/processes/:name" element={<ProcessOverview />} />
        </Routes>
      </MemoryRouter>,
    )

    await waitFor(() => {
      expect(screen.getByText(/no steps defined/i)).toBeInTheDocument()
    })

    expect(screen.queryByText('State machine')).not.toBeInTheDocument()
  })

  it('renders both the step UI and the state machine card when steps exist', async () => {
    /**
     * GIVEN a process with 5 steps and 5 transitions
     * WHEN the page loads
     * THEN the "State machine" heading IS in the document AND the first step's humanized
     * state appears (the step UI still renders).
     */
    const steps = [
      { state: 'created', fields: [], documents: [], gates: [], kpis: [] },
      { state: 'validated', fields: [], documents: [], gates: [], kpis: [] },
      { state: 'approved', fields: [], documents: [], gates: [], kpis: [] },
      { state: 'executed', fields: [], documents: [], gates: [], kpis: [] },
      { state: 'closed', fields: [], documents: [], gates: [], kpis: [] },
    ]
    const transitions = [
      { from: 'created', to: 'validated' },
      { from: 'validated', to: 'approved' },
      { from: 'approved', to: 'executed' },
      { from: 'executed', to: 'closed' },
      { from: 'closed', to: 'archived' },
    ]
    const process = {
      name: 'five_step_process',
      process: 'five_step_process',
      states: steps.map((s) => s.state),
      transitions,
      mermaid: 'graph LR',
      steps,
    }
    ;(api.getProcess as ReturnType<typeof vi.fn>).mockResolvedValue(process)

    render(
      <MemoryRouter initialEntries={['/processes/five_step_process']}>
        <Routes>
          <Route path="/processes/:name" element={<ProcessOverview />} />
        </Routes>
      </MemoryRouter>,
    )

    await waitFor(() => {
      expect(api.getProcess).toHaveBeenCalledWith('five_step_process')
    })

    expect(screen.getByText('State machine')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: humanize(steps[0].state) })).toBeInTheDocument()
  })
})
