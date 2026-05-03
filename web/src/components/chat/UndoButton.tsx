import { Undo2 } from 'lucide-react';

interface UndoButtonProps {
  onClick: () => void;
}

export function UndoButton({ onClick }: UndoButtonProps) {
  return (
    <button
      onClick={onClick}
      className="opacity-0 transition-opacity duration-150 group-hover:opacity-100"
      title="Undo to this message"
      aria-label="Undo to this message"
    >
      <Undo2 className="h-3.5 w-3.5 text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300" />
    </button>
  );
}
