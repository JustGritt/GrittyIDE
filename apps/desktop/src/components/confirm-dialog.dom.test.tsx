/** @vitest-environment jsdom */
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ConfirmDialog } from './confirm-dialog';

/**
 * The shared confirmation's policy, as a keyboard user meets it (T102-U).
 *
 * A DOM rather than static markup: focus and Escape are the whole point, and
 * neither exists in a string.
 */

afterEach(cleanup);

function mount() {
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  render(
    <ConfirmDialog
      title="Discard changes to a.ts?"
      confirmLabel="Discard changes"
      onConfirm={onConfirm}
      onCancel={onCancel}
    >
      <p>The changes are stashed first.</p>
    </ConfirmDialog>,
  );
  return { onConfirm, onCancel };
}

describe('the confirm dialog', () => {
  it('is an alert dialog named by its title and described by its body', () => {
    mount();
    const dialog = screen.getByRole('alertdialog');
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    const title = document.getElementById(dialog.getAttribute('aria-labelledby') ?? '');
    const body = document.getElementById(dialog.getAttribute('aria-describedby') ?? '');
    expect(title?.textContent).toBe('Discard changes to a.ts?');
    expect(body?.textContent).toBe('The changes are stashed first.');
  });

  it('puts focus on Cancel, so Enter on a dialog that just appeared keeps what the user has', () => {
    const { onConfirm, onCancel } = mount();
    expect(document.activeElement?.textContent).toBe('Cancel');
    fireEvent.click(document.activeElement as HTMLElement);
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it('cancels on Escape from anywhere inside it', () => {
    const { onConfirm, onCancel } = mount();
    fireEvent.keyDown(screen.getByText('The changes are stashed first.'), { key: 'Escape' });
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it('confirms only from its own button', () => {
    const { onConfirm } = mount();
    fireEvent.click(screen.getByRole('button', { name: 'Discard changes' }));
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });
});
