/** @vitest-environment jsdom */
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import { type SidebarTab, SidebarTabs, sidebarPanelId, sidebarTabId } from './sidebar-tabs';

/**
 * The sidebar header, as a keyboard user meets it (T100-U).
 *
 * A DOM rather than static markup, because what is being asserted is focus and
 * key handling — neither of which exists in a string. `tabIndex` and
 * `aria-selected` can be read off markup; *which element the browser focused
 * after ArrowRight* cannot.
 */

function Harness({ initial = 'files' as SidebarTab }: { initial?: SidebarTab }) {
  const [tab, setTab] = useState<SidebarTab>(initial);
  return (
    <>
      <SidebarTabs tab={tab} onTabChange={setTab} />
      <div role="tabpanel" id={sidebarPanelId(tab)} aria-labelledby={sidebarTabId(tab)}>
        {tab === 'files' ? 'the tree' : 'the changes'}
      </div>
    </>
  );
}

afterEach(() => {
  cleanup();
});

describe('the sidebar tablist', () => {
  it('is a tablist with the two sections artboard 9a drew', () => {
    render(<Harness />);
    expect(screen.getByRole('tablist')).toBeTruthy();
    expect(screen.getAllByRole('tab').map((tab) => tab.textContent)).toEqual(['Files', 'Changes']);
  });

  it('marks exactly one tab selected and points it at the panel that exists', () => {
    render(<Harness />);
    const [files, changes] = screen.getAllByRole('tab');
    expect(files?.getAttribute('aria-selected')).toBe('true');
    expect(changes?.getAttribute('aria-selected')).toBe('false');
    expect(files?.getAttribute('aria-controls')).toBe(sidebarPanelId('files'));
    // The unselected panel is not in the document, so its tab points at nothing
    // rather than at a broken reference.
    expect(changes?.getAttribute('aria-controls')).toBeNull();
  });

  it('keeps one stop in the tab sequence, the roving tabindex the APG asks for', () => {
    render(<Harness />);
    const [files, changes] = screen.getAllByRole('tab');
    expect(files?.getAttribute('tabindex')).toBe('0');
    expect(changes?.getAttribute('tabindex')).toBe('-1');
  });

  it('moves selection and focus together on ArrowRight and ArrowLeft', () => {
    render(<Harness />);
    const tablist = screen.getByRole('tablist');
    fireEvent.keyDown(tablist, { key: 'ArrowRight' });
    expect(screen.getByRole('tab', { name: 'Changes' }).getAttribute('aria-selected')).toBe('true');
    // Automatic activation: the panel changed with the selection, and focus
    // followed, so a keyboard user is not left on a tab with tabindex -1.
    expect(document.activeElement).toBe(screen.getByRole('tab', { name: 'Changes' }));
    expect(screen.getByRole('tabpanel').textContent).toBe('the changes');

    fireEvent.keyDown(tablist, { key: 'ArrowLeft' });
    expect(screen.getByRole('tab', { name: 'Files' }).getAttribute('aria-selected')).toBe('true');
    expect(document.activeElement).toBe(screen.getByRole('tab', { name: 'Files' }));
  });

  it('wraps at both ends rather than stopping', () => {
    render(<Harness />);
    const tablist = screen.getByRole('tablist');
    fireEvent.keyDown(tablist, { key: 'ArrowLeft' });
    expect(screen.getByRole('tab', { name: 'Changes' }).getAttribute('aria-selected')).toBe('true');
    fireEvent.keyDown(tablist, { key: 'ArrowRight' });
    expect(screen.getByRole('tab', { name: 'Files' }).getAttribute('aria-selected')).toBe('true');
  });

  it('answers Home and End as well as the arrows', () => {
    render(<Harness />);
    const tablist = screen.getByRole('tablist');
    fireEvent.keyDown(tablist, { key: 'End' });
    expect(screen.getByRole('tab', { name: 'Changes' }).getAttribute('aria-selected')).toBe('true');
    fireEvent.keyDown(tablist, { key: 'Home' });
    expect(screen.getByRole('tab', { name: 'Files' }).getAttribute('aria-selected')).toBe('true');
  });

  it('leaves keys it does not own to whatever else wants them', () => {
    render(<Harness />);
    const tablist = screen.getByRole('tablist');
    const event = new KeyboardEvent('keydown', { key: 'g', bubbles: true, cancelable: true });
    fireEvent(tablist, event);
    // ⌥G reaches the Agents section through a document listener; a tablist that
    // swallowed unrelated keys would be the sixth hand-gated listener fighting
    // the fifth.
    expect(event.defaultPrevented).toBe(false);
    expect(screen.getByRole('tab', { name: 'Files' }).getAttribute('aria-selected')).toBe('true');
  });

  it('switches on a click too', () => {
    render(<Harness />);
    fireEvent.click(screen.getByRole('tab', { name: 'Changes' }));
    expect(screen.getByRole('tabpanel').textContent).toBe('the changes');
  });
});
