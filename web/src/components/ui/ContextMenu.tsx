import { useTranslation } from "react-i18next";

import { Menu } from "./Menu";

export interface ContextMenuItem {
  type?: "item" | "separator";
  label?: string;
  icon?: React.ReactNode;
  shortcut?: string;
  translated?: boolean;
  disabled?: boolean;
  danger?: boolean;
  onClick?: () => void;
}

interface ContextMenuProps {
  x: number;
  y: number;
  items: ContextMenuItem[];
  onClose: () => void;
}

export function ContextMenu({ x, y, items, onClose }: ContextMenuProps) {
  const { t } = useTranslation();

  return (
    <Menu.Root open onOpenChange={(open) => !open && onClose()}>
      <Menu.Trigger asChild>
        <span className="ui-context-menu-anchor" style={{ left: x, top: y }} aria-hidden="true" />
      </Menu.Trigger>
      <Menu.Content side="bottom" align="start" sideOffset={0} collisionPadding={8}>
        {items.map((item, i) =>
          item.type === "separator" ? (
            <Menu.Separator key={`separator-${i}`} />
          ) : (
            <Menu.Item
              key={`item-${i}`}
              disabled={item.disabled}
              data-tone={item.danger ? "danger" : undefined}
              onSelect={() => item.onClick?.()}
            >
              {item.icon && <span className="ui-menu-item-icon">{item.icon}</span>}
              <span className="ui-menu-item-label">
                {item.label ? (item.translated === false ? item.label : t(item.label)) : null}
              </span>
              {item.shortcut && <span className="ui-menu-shortcut">{item.shortcut}</span>}
            </Menu.Item>
          ),
        )}
      </Menu.Content>
    </Menu.Root>
  );
}
