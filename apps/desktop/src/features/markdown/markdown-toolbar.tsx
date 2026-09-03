import { VIEW_MODES, type ViewMode } from './view-modes';

const LABELS: Record<ViewMode, string> = { source: 'Source', split: 'Split', preview: 'Preview' };

export function MarkdownToolbar({
  mode,
  onMode,
  instructionFile,
  notice,
  onDismissNotice,
}: {
  mode: ViewMode;
  onMode: (mode: ViewMode) => void;
  instructionFile: boolean;
  notice: string | null;
  onDismissNotice: () => void;
}) {
  return (
    <div className="mdbar">
      {VIEW_MODES.map((candidate) => (
        <button
          key={candidate}
          type="button"
          className="mdbar__mode"
          aria-pressed={mode === candidate}
          onClick={() => onMode(candidate)}
        >
          {LABELS[candidate]}
        </button>
      ))}
      {instructionFile && (
        <span
          className="mdbar__badge"
          title="README.md and AGENTS.md are recognized as instruction files; nothing vendor-specific is attached to them."
        >
          instruction file
        </span>
      )}
      {notice && (
        <span className="mdbar__notice" role="status">
          {notice}
          <button type="button" onClick={onDismissNotice}>
            Dismiss
          </button>
        </span>
      )}
    </div>
  );
}
