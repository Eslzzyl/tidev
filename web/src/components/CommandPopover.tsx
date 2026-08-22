import type { CommandSuggestion } from "../commands";

interface CommandPopoverProps {
  suggestions: CommandSuggestion[];
  selectedIndex: number;
  onSelectedIndexChange: (index: number) => void;
  onSelect: (suggestion: CommandSuggestion) => void;
}

export function CommandPopover({
  suggestions,
  selectedIndex,
  onSelectedIndexChange,
  onSelect,
}: CommandPopoverProps) {
  if (suggestions.length === 0) return null;

  return (
    <div className="composer-popover command-popover" role="listbox">
      {suggestions.map((suggestion, index) => (
        <button
          key={suggestion.spec.name}
          type="button"
          className={
            index === selectedIndex
              ? "composer-option command-option selected"
              : "composer-option command-option"
          }
          role="option"
          aria-selected={index === selectedIndex}
          onMouseDown={(event) => event.preventDefault()}
          onMouseEnter={() => onSelectedIndexChange(index)}
          onClick={() => onSelect(suggestion)}
        >
          <span className="command-option-main">
            <strong>/{suggestion.spec.name}</strong>
            <small>{suggestion.spec.description}</small>
          </span>
          <small className="command-option-usage">{suggestion.spec.usage}</small>
        </button>
      ))}
    </div>
  );
}
