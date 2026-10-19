// @vitest-environment jsdom
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { relayAccount } from '../model/fixtures';
import { toRelayView } from '../model/relay';
import { RelayCard } from './RelayCard';

afterEach(cleanup);

const ADDR = 'C9eFRXy11aT8JqcyZPvEGqsor3pmrAyPDBmg6LCcGvWS';

describe('RelayCard', () => {
  it('shows tip, Finality Depth and Final Block Tree size for a running relay, with no alert', () => {
    render(<RelayCard label="Zcash testnet" address={ADDR} view={toRelayView(relayAccount())} error={undefined} />);
    expect(screen.getByTestId('tip-height').textContent).toBe('4,517,300');
    expect(screen.getByText('Running')).toBeTruthy();
    expect(screen.getByText('199 blocks')).toBeTruthy();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('makes a Halt impossible to miss: alert, reason, halt height, red card', () => {
    const view = toRelayView(relayAccount({ halted: 1, haltHeight: 4_517_333 }));
    const { container } = render(<RelayCard label="Zcash testnet" address={ADDR} view={view} error={undefined} />);
    const alert = screen.getByRole('alert');
    expect(alert.textContent).toContain('HALTED');
    expect(alert.textContent).toContain('reorganised past a final block');
    expect(alert.textContent).toContain('4,517,333');
    expect(screen.getAllByText('HALTED').length).toBeGreaterThan(0);
    expect(container.querySelector('.relay.is-halted')).not.toBeNull();
    expect(screen.queryByText('Running')).toBeNull();
  });
});
