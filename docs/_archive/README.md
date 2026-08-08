# Archive — superseded source material

These files are the **pre-reorganization** drafts. They are kept for provenance
only and are **not authoritative**. Nothing in `docs/` should link to them.

| File | Why archived |
| --- | --- |
| `TaskForge_Product_and_System_Architecture.docx` | The Rust-era master draft. Its content is the seed for the current `docs/` set. Superseded by [00-README](../00-README.md). |
| `TaskForge_Product_and_System_Architecture (1).docx` | Older Java/Spring-era export. **Wrong stack** — superseded by ADR-001. |
| `TaskForge_Product_and_Architecture.md` | Markdown master that was never updated after the Rust decision; still said "Java LTS, Spring Boot, Flyway, Gradle". **Wrong stack.** |
| `02_Domain_RBAC_Workflow.md` | Verbatim section copy of the stale master. |
| `03_Lightweight_Client_Architecture.md` | Verbatim section copy of the stale master. |
| `04_Backend_and_Data_Architecture.md` | Verbatim section copy of the stale master. |
| `05_Plugin_Security_Deployment.md` | Verbatim section copy of the stale master. |

Two byte-identical `(1)` duplicates were deleted outright.

## The lesson these encode

The `0X_*.md` files were **copies**, not references. When the backend decision
changed from Java to Rust, the `.docx` was updated and the four markdown copies
were not. Six files drifted from one decision.

This is why the current `docs/` set has a **single owner per fact** and
cross-references by number instead of restating
([16-DOCUMENTATION-MAINTENANCE](../16-DOCUMENTATION-MAINTENANCE.md)).
