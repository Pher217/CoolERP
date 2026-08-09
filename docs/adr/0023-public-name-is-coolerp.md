# ADR-023 — Public name is "CoolERP"

- **Status:** Accepted (2026-08-05) · **supersedes ADR-010**, closing the ADR-001 → ADR-010 thread
- **Deciders:** Philippe Hermann

## Context
The project was named "OpenERP" — which was **Odoo's product name from 2009 until
June 2014**. The naming question was decided twice before and reversed once:

- **ADR-001 (2026-05-29)** found low legal risk but high brand/SEO collision, and
  recommended renaming.
- **ADR-010 (2026-06-01)** reversed that and kept "OpenERP", judging the collision
  "a marketing problem to manage, not an architectural one."

Re-examined on 2026-08-05 with all prior reasoning in view. A live search for
"OpenERP" returned results that were *entirely* Odoo-owned or Odoo-explaining
content. The practical effect is not merely poor discoverability but an
adversarial one: brand searches would funnel traffic to the project's largest
competitor. Separately, Odoo publishes no public trademark-use policy for the
name — that is *unresolved*, not *safe*.

The deciding factor is cost asymmetry over time: renaming while the repo is
private, with zero users, stars, or inbound links, is a text substitution.
Every one of those denominators becomes non-zero at launch, and the cost then
becomes permanent.

## Decision
The public product and brand name is **CoolERP**.

Candidates were re-verified first-hand against crates.io, npm, PyPI, and GitHub:

| Candidate | Verdict |
|---|---|
| **CoolERP** | Free on all registries; no ERP collision. **Chosen.** |
| GoodERP | **Rejected** — taken by [`osbzr/gooderp_addons`](https://github.com/osbzr/gooderp_addons), an active 1,300★ open-source ERP *itself built on Odoo*. |
| Oledge | Free, but carries no category keyword. Runner-up. |

"ERP" is retained deliberately as a discovery asset: a coinage carries no meaning
and must be marketed into existence.

## Consequences
- Text substitution across 38 files. `cargo fmt`, `cargo build --workspace`, and
  `cargo test --all` (131 passed / 0 failed) all verified green afterwards.
- **Crate names `ol-*` and the `ol` CLI binary are kept unchanged.** The prefix no
  longer expands to anything under "CoolERP" — a real if minor wart, accepted in
  preference to an invasive, build-risky crate rename. Revisit only if it
  confuses contributors.
- **Outstanding operator action:** rename the GitHub repository, after which the
  repo URLs in these docs get a follow-up sweep. Until then the docs carry the
  new name and the old URL.
