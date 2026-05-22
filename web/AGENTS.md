# tidev web frontend

React 19 + TypeScript + Vite + Zustand + Tailwind CSS SPA.

## Project Structure

```
web/
├── src/
│   ├── api/           # API client layer (client.ts, sse.ts)
│   ├── components/    # React components
│   │   ├── chat/      # Chat-related components
│   │   ├── layout/    # Layout components
│   │   ├── renderers/ # Markdown/Diff/Code/Chart renderers
│   │   ├── settings/  # Settings page components
│   │   ├── ui/        # Shared UI components
│   │   └── views/     # Top-level view components
│   ├── hooks/         # Custom React Hooks (useSSE, useSmartInput, etc.)
│   ├── lib/           # Utilities (router, codemirror config)
│   ├── stores/        # Zustand state management
│   ├── test/          # Test setup (setup.ts)
│   ├── types/         # TypeScript type definitions
│   ├── utils/         # Pure utility functions (format, round)
│   ├── commands.ts    # Slash command definitions & fuzzy matching
│   ├── App.tsx        # Root application component
│   └── main.tsx       # Entry point
├── public/            # Static assets
└── dist/              # Build output
```

## Commands

```bash
pnpm dev              # Start dev server (localhost:5173)
pnpm build            # TypeScript check + production build
pnpm lint             # ESLint
pnpm format           # Prettier
pnpm test             # Run all tests (vitest run)
pnpm test:watch       # Watch mode
pnpm test:coverage    # With coverage report
```
