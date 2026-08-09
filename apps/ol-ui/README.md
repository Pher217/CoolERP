# ol-ui — CoolERP Web App

React 19 + Vite + TypeScript browser SPA for the CoolERP REST API.

## Prerequisites

- Node 20+ / npm 10+

## Getting started

```bash
cd apps/ol-ui
npm install
npm run dev        # development server at http://localhost:5173
```

## Build

```bash
npm run build      # type-check + bundle → dist/
```

## Lint

```bash
npm run lint
```

## Tests

```bash
npm run test       # vitest — money formatting unit tests
```

## API codegen

Regenerate `src/api/schema.d.ts` from the live OpenAPI spec (requires ol-api running):

```bash
npm run gen:api
```

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `VITE_API_BASE` | `http://localhost:3000` | Base URL of the ol-api REST server |

Create a `.env.local` file in this directory to override:

```
VITE_API_BASE=http://localhost:3000
```
