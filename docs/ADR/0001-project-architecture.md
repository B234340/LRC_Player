# ADR 0001: Project Architecture

## Status

Accepted

## Context

`LRC_Player` is a desktop music player designed with a clean architecture approach to separate business logic from UI and infrastructure details.

The codebase is structured into:

- `domain/` — pure domain models and business services without I/O.
- `application/` — use cases, DTOs, view models, and abstract ports for external dependencies.
- `infrastructure/` — concrete adapters for persistence, audio output, decoding, scanning, and other external systems.
- `presentation/` — UI implementation using `eframe`/`egui`.
- `shared/` — common utilities and IPC support.

## Decision

The project will maintain strict boundaries between layers:

- Domain logic remains independent of I/O and platform specifics.
- Application use cases interact with external systems through traits defined in `application/ports`.
- Infrastructure provides concrete implementations for those ports.
- Presentation builds view models from application DTOs and invokes use cases through application services.

## Consequences

- Easier testing and separation of concerns.
- Clear migration path for adding new backends or replacing implementation details.
- Some additional boilerplate is required for mapping between DTOs, domain models, and view models.
