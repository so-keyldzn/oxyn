# Interface gallery and demos

Native macOS captures recorded on 2026-10-03 with **Oxyn 0.0.3**, a temporary
workspace and the synthetic SQLite dataset in [seed.sql](seed.sql). Company
names, orders and revenue are fictional. No external database or AI provider
is involved.

## Screenshots

### SQL workspace

A revenue query joins customers and paid orders, groups by country and returns
results in the grid. The exact query is in [revenue.sql](revenue.sql).

![SQL editor and revenue results](screenshots/sql-workspace.jpg)

### Table explorer

The catalog and the Data tab show the demo orders.

![Orders in the table explorer](screenshots/table-explorer.jpg)

### Schema inspection

The Structure tab shows columns read from the SQLite catalog.

![Order columns in the Structure tab](screenshots/table-structure.jpg)

## Videos

| Demo | Duration | Watch | What it shows |
|---|---|---|---|
| Explore a database | 31 s | [MP4](videos/explore-database.mp4) | Open orders, browse data, inspect Structure, DDL and Indexes. |
| Run a revenue query | 21 s | [MP4](videos/run-query.mp4) | Enter SQL, run it explicitly and inspect the result grid. |

The videos are silent H.264 screen recordings (1920 × 1286, 30 fps). Idle
pauses were trimmed; the interactions and results are real. Screenshots keep
the original 2560 × 1640 pixels. Download the MP4 links if your
Markdown viewer does not offer playback. Captures demonstrate these specific
local workflows; they do not certify other drivers or AI features.

## Animated walkthroughs

One short video per feature, plus an overview that chains the essentials.
These are **not screen recordings**: the interface is rebuilt from the same
shadcn/Base UI primitives, OKLCH tokens and fonts as the application, then
animated frame by frame with Remotion. They follow Oxyn 0.0.8 and the
[seed.sql](seed.sql) dataset, but some labels, questions and AI answers were
written for the videos; the recordings above remain the reference for what the
application actually displays.

| Walkthrough | Duration | Watch | What it shows |
|---|---|---|---|
| Overview | 46 s | [MP4](videos/motion/overview.mp4) | Exploration, query and guardrails, chained. |
| Intro | 4 s | [MP4](videos/motion/intro.mp4) | The mark, the name, the promise. |
| Connections | 16 s | [MP4](videos/motion/connections.mp4) | Engine choice, form, environment, test, opening the workspace. |
| Explore a database | 11 s | [MP4](videos/motion/explore-database.mp4) | The catalog unfolds, `orders` opens, then its structure and DDL. |
| Relations | 16 s | [MP4](videos/motion/relations.mp4) | Indexes, constraints, incoming and outgoing relations, ER diagram. |
| Run a query | 14 s | [MP4](videos/motion/run-query.mp4) | The revenue query is typed, ⌘↵, rows arrive, the query is saved. |
| Result tools | 16 s | [MP4](videos/motion/result-tools.mp4) | Search in loaded rows, columns, density, value inspection, export. |
| Charts | 15 s | [MP4](videos/motion/charts.mp4) | A result becomes a chart, only in the shapes the data allows. |
| Library | 15 s | [MP4](videos/motion/library.mp4) | Saved queries, modified drafts, kept results. |
| Command palette | 14 s | [MP4](videos/motion/command-palette.mp4) | ⌘K palette, ⌘P quick open, ⌘/ shortcut sheet. |
| Object operations | 16 s | [MP4](videos/motion/object-operations.mp4) | Rename… and Drop… go through a review dialog; the name is typed in production. |
| Guardrails | 15 s | [MP4](videos/motion/guardrails.mp4) | Permanent markers, native production-write dialog, agent proposal kept as text. |
| Assistant | 19 s | [MP4](videos/motion/assistant.mp4) | Object mention, plan, tool calls, a sample and its native confirmation, sources. |
| AI privacy | 16 s | [MP4](videos/motion/ai-privacy.mp4) | Local and remote providers, privacy tiers per connection. |
| Recovery | 15 s | [MP4](videos/motion/recovery.mp4) | Quitting with an open transaction, drafts restored after a crash. |
| Outro | 4 s | [MP4](videos/motion/outro.mp4) | The signature and the repository address. |

H.264, 1920 × 1080, 30 fps, English captions, with a soundtrack: "The Inner
Space" by Art Flower via Free Music Archive and interface sound effects from
Freesound, Kenney and the Remotion library, all under CC0 1.0.

## Capture provenance

The installed application is not a build verified against this checkout.
The original installed executable has SHA-256
`1d85505c37e9daceb4c845458602057ab6fbbeac8785fb6e4f48b4f0292e1d99`.

For capture isolation, a disposable copy of the installed app used a distinct
bundle identifier and an ad-hoc local signature. Application code and web
assets were unchanged; the fingerprint above identifies the executable before
re-signing.

## Reproduce the scenes

From the repository root, create a fresh disposable database:

```sh
demo_dir=$(mktemp -d /tmp/oxyn-demo.XXXXXX)
sqlite3 "$demo_dir/studio.sqlite" < assets/demo/seed.sql
printf '%s\n' "$demo_dir/studio.sqlite"
open -n /Applications/Oxyn.app --args --temporary-workspace
```

These recordings use the installed macOS application. To try the same workflow
with the source checkout instead, use `make desktop-dev`.

In the temporary application:

1. Create a SQLite connection named **Studio Commerce** using the printed path.
   Set its environment to **Development** and enable read-only access.
2. Connect, expand the catalog and open **orders**. Capture the **Data** and
   **Structure** tabs; include **DDL** in the exploration recording.
3. Open an SQL console, enter [revenue.sql](revenue.sql), select the SQL
   explicitly, then click **Run**. Capture the editor and completed result grid.

Capture only the demo window. Keep unrelated windows, notifications, personal
paths and credentials out of the frame. Do not record an existing workspace.
Use the original screen pixels for screenshots and encode videos as H.264 MP4
without an audio track. Check the output visually before replacing these files.
The database and raw recordings are local working files, not repository assets.
