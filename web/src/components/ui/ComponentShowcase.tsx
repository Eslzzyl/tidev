import { useEffect, useState } from "react";
import {
  Check,
  ChevronDown,
  CircleHelp,
  Info,
  MoreHorizontal,
  Search,
  Sparkles,
} from "lucide-react";

import {
  Badge,
  Button,
  Checkbox,
  Collapsible,
  Dialog,
  Alert,
  Field,
  IconButton,
  Input,
  Menu,
  Popover,
  Select,
  Slider,
  Spinner,
  Switch,
  Table,
  Tabs,
  Textarea,
  Tooltip,
} from ".";

const selectOptions = [
  { value: "system", label: "Use browser language" },
  { value: "en", label: "English" },
  { value: "zh-CN", label: "简体中文" },
];

const modelGroups = [
  {
    label: "OpenAI",
    options: [
      { value: "gpt-5", label: "GPT-5" },
      { value: "gpt-5-mini", label: "GPT-5 Mini" },
    ],
  },
  {
    label: "Local",
    options: [
      { value: "qwen", label: "Qwen 3 32B" },
      { value: "disabled", label: "Unavailable model", disabled: true },
    ],
  },
];

const paletteOptions = [
  { value: "ocean", label: "Ocean", color: "#2563eb" },
  { value: "violet", label: "Violet", color: "#7c3aed" },
  { value: "teal", label: "Teal", color: "#0f766e" },
  { value: "emerald", label: "Emerald", color: "#059669" },
  { value: "amber", label: "Amber", color: "#d97706" },
  { value: "rose", label: "Rose", color: "#e11d48" },
] as const;

type PaletteName = (typeof paletteOptions)[number]["value"];

export function ComponentShowcase() {
  const [language, setLanguage] = useState("system");
  const [model, setModel] = useState("gpt-5");
  const [enabled, setEnabled] = useState(true);
  const [notificationsEnabled, setNotificationsEnabled] = useState(true);
  const [editorScale, setEditorScale] = useState([75]);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [palette, setPalette] = useState<PaletteName>("ocean");

  useEffect(() => {
    const root = document.documentElement;
    const previousPalette = root.dataset.uiPalette;
    root.dataset.uiPalette = palette;
    return () => {
      if (previousPalette) {
        root.dataset.uiPalette = previousPalette;
      } else {
        delete root.dataset.uiPalette;
      }
    };
  }, [palette]);

  return (
    <div className="ui-showcase-route">
      <div className="ui-showcase-header">
        <div>
          <p className="ui-eyebrow">Tidev UI</p>
          <h1>Component showcase</h1>
          <p>Buttons, fields, menus, overlays, and common interaction states.</p>
        </div>
      </div>

      <div className="ui-showcase-grid">
        <ShowcaseSection title="Color themes" description="Choose a color theme for the interface.">
          <div className="ui-palette-grid">
            {paletteOptions.map((option) => (
              <button
                key={option.value}
                type="button"
                className="ui-palette-option"
                data-selected={palette === option.value || undefined}
                aria-pressed={palette === option.value}
                onClick={() => setPalette(option.value)}
              >
                <span className="ui-palette-swatch" style={{ background: option.color }} />
                <span>{option.label}</span>
              </button>
            ))}
          </div>
        </ShowcaseSection>

        <ShowcaseSection
          title="Buttons"
          description="Primary, secondary, quiet, destructive, and loading states."
        >
          <div className="ui-showcase-row">
            <Button variant="primary" leadingIcon={<Sparkles size={14} />}>
              Primary action
            </Button>
            <Button variant="secondary">Secondary</Button>
            <Button variant="ghost">Ghost</Button>
            <Button variant="danger">Delete</Button>
            <Button loading>Saving</Button>
            <IconButton label="More actions">
              <MoreHorizontal size={16} />
            </IconButton>
          </div>
          <div className="ui-showcase-row">
            <Button size="sm">Small</Button>
            <Button size="md">Medium</Button>
            <Button size="lg">Large</Button>
            <Button disabled>Disabled</Button>
          </div>
        </ShowcaseSection>

        <ShowcaseSection
          title="Form controls"
          description="Labels, descriptions, errors, and controls share one field rhythm."
        >
          <div className="ui-showcase-form-grid">
            <Field
              label="Workspace name"
              description="Shown in the session sidebar."
              htmlFor="showcase-name"
            >
              <Input id="showcase-name" placeholder="My workspace" />
            </Field>
            <Field label="Language" htmlFor="showcase-language">
              <Select
                id="showcase-language"
                value={language}
                onValueChange={setLanguage}
                options={selectOptions}
                placeholder="Choose a language"
              />
            </Field>
            <Field label="Model" description="Models grouped by provider." htmlFor="showcase-model">
              <Select
                id="showcase-model"
                value={model}
                onValueChange={setModel}
                groups={modelGroups}
              />
            </Field>
            <Field
              label="Invalid field"
              error="This value needs attention."
              htmlFor="showcase-invalid"
            >
              <Input id="showcase-invalid" defaultValue="Unsupported value" invalid />
            </Field>
          </div>
          <Field label="Instructions" htmlFor="showcase-instructions">
            <Textarea
              id="showcase-instructions"
              placeholder="Add a short instruction..."
              rows={3}
            />
          </Field>
          <div className="ui-showcase-row ui-showcase-muted-row">
            <Input placeholder="Disabled input" disabled />
            <Button leadingIcon={<Search size={14} />}>Search</Button>
          </div>
        </ShowcaseSection>

        <ShowcaseSection title="Menu and popover" description="Actions and contextual information.">
          <div className="ui-showcase-row">
            <Menu.Root>
              <Menu.Trigger asChild>
                <Button trailingIcon={<ChevronDown size={14} />}>Open menu</Button>
              </Menu.Trigger>
              <Menu.Content align="start">
                <Menu.Label>Actions</Menu.Label>
                <Menu.Item>
                  <Check size={14} /> Run task
                </Menu.Item>
                <Menu.Item>Duplicate session</Menu.Item>
                <Menu.Separator />
                <Menu.Item>Delete session</Menu.Item>
              </Menu.Content>
            </Menu.Root>

            <Popover.Root>
              <Popover.Trigger asChild>
                <Button variant="ghost" leadingIcon={<Info size={14} />}>
                  Show details
                </Button>
              </Popover.Trigger>
              <Popover.Content align="start">
                <strong>Popover surface</strong>
                <p className="ui-popover-copy">Additional information appears here.</p>
              </Popover.Content>
            </Popover.Root>

            <Dialog.Root>
              <Dialog.Trigger asChild>
                <Button variant="secondary">Open dialog</Button>
              </Dialog.Trigger>
              <Dialog.Content>
                <Dialog.Header>
                  <Dialog.Title>Confirm workspace action</Dialog.Title>
                  <Dialog.Description>Confirm the selected workspace action.</Dialog.Description>
                </Dialog.Header>
                <Dialog.Footer>
                  <Dialog.Close asChild>
                    <Button variant="ghost">Cancel</Button>
                  </Dialog.Close>
                  <Dialog.Close asChild>
                    <Button variant="primary">Continue</Button>
                  </Dialog.Close>
                </Dialog.Footer>
              </Dialog.Content>
            </Dialog.Root>
          </div>
        </ShowcaseSection>

        <ShowcaseSection
          title="Selection and feedback"
          description="Selection controls, guidance, disclosure, and status messages."
        >
          <div className="ui-showcase-row">
            <label className="ui-showcase-check-row">
              <Checkbox
                checked={notificationsEnabled}
                onCheckedChange={(value) => setNotificationsEnabled(value === true)}
                aria-label="Enable desktop notifications"
              />
              <span>Desktop notifications</span>
            </label>

            <Tooltip.Root>
              <Tooltip.Trigger asChild>
                <IconButton label="Show help">
                  <CircleHelp size={16} />
                </IconButton>
              </Tooltip.Trigger>
              <Tooltip.Content>Keyboard shortcuts are available in every view.</Tooltip.Content>
            </Tooltip.Root>
          </div>

          <div className="ui-showcase-slider">
            <div className="ui-showcase-slider-label">
              <span>Editor scale</span>
              <strong>{editorScale[0]}%</strong>
            </div>
            <Slider
              value={editorScale}
              onValueChange={setEditorScale}
              min={50}
              max={150}
              step={5}
              aria-label="Editor scale"
            />
          </div>

          <Alert tone="info" title="Information">
            Changes are applied to the current workspace.
          </Alert>
          <Alert tone="success" title="Saved">
            Preferences are synchronized.
          </Alert>

          <Collapsible.Root
            open={detailsOpen}
            onOpenChange={setDetailsOpen}
            className="ui-showcase-disclosure"
          >
            <Collapsible.Trigger>
              <ChevronDown size={14} aria-hidden="true" />
              Advanced details
            </Collapsible.Trigger>
            <Collapsible.Content>
              Additional settings can be revealed without leaving the current context.
            </Collapsible.Content>
          </Collapsible.Root>
        </ShowcaseSection>

        <ShowcaseSection title="Table" description="Scrollable, themed data presentation.">
          <Table.Root>
            <Table.Header>
              <Table.Row>
                <Table.Head>Session</Table.Head>
                <Table.Head>Status</Table.Head>
                <Table.Head>Updated</Table.Head>
              </Table.Row>
            </Table.Header>
            <Table.Body>
              <Table.Row>
                <Table.Cell>Workspace setup</Table.Cell>
                <Table.Cell>Ready</Table.Cell>
                <Table.Cell>Just now</Table.Cell>
              </Table.Row>
              <Table.Row>
                <Table.Cell>Code review</Table.Cell>
                <Table.Cell>In progress</Table.Cell>
                <Table.Cell>2 min ago</Table.Cell>
              </Table.Row>
            </Table.Body>
          </Table.Root>
        </ShowcaseSection>

        <ShowcaseSection
          title="States and navigation"
          description="Selection, status, and navigation states."
        >
          <div className="ui-showcase-state-grid">
            <div className="ui-showcase-state-card">
              <div className="ui-showcase-state-heading">
                <span>Subagents</span>
                <Badge tone={enabled ? "success" : "neutral"}>{enabled ? "Enabled" : "Off"}</Badge>
              </div>
              <Switch
                checked={enabled}
                onCheckedChange={setEnabled}
                aria-label="Enable subagents"
              />
            </div>
            <div className="ui-showcase-state-card">
              <span className="ui-field-label">Sync status</span>
              <div className="ui-showcase-status">
                <Spinner />
                <span>Saving preferences...</span>
              </div>
            </div>
            <div className="ui-showcase-state-card">
              <span className="ui-field-label">Semantic tones</span>
              <div className="ui-showcase-row">
                <Badge>Neutral</Badge>
                <Badge tone="success">Success</Badge>
                <Badge tone="warning">Warning</Badge>
                <Badge tone="danger">Danger</Badge>
              </div>
            </div>
          </div>
          <Tabs.Root defaultValue="overview">
            <Tabs.List>
              <Tabs.Trigger value="overview">Overview</Tabs.Trigger>
              <Tabs.Trigger value="activity">Activity</Tabs.Trigger>
              <Tabs.Trigger value="settings">Settings</Tabs.Trigger>
            </Tabs.List>
            <Tabs.Content value="overview">Summary content.</Tabs.Content>
            <Tabs.Content value="activity">Recent activity.</Tabs.Content>
            <Tabs.Content value="settings">Preferences.</Tabs.Content>
          </Tabs.Root>
        </ShowcaseSection>
      </div>
    </div>
  );
}

function ShowcaseSection({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: React.ReactNode;
}) {
  return (
    <section className="ui-showcase-section">
      <div className="ui-showcase-section-header">
        <div>
          <h2>{title}</h2>
          <p>{description}</p>
        </div>
      </div>
      <div className="ui-showcase-section-body">{children}</div>
    </section>
  );
}
