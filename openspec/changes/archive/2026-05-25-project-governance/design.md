## Context

Proyecto Rust greenfield con un cambio fundacional en curso (`rust-cqrs-framework`). Sin gobierno, las decisiones de arquitectura y testing se tomarían ad-hoc en cada spec, generando inconsistencia. Este cambio establece las reglas antes de que el proyecto escale.

**Stakeholders**: Todo desarrollador que cree specs o implemente código en este proyecto.

## Goals / Non-Goals

**Goals:**
- Definir reglas de arquitectura que toda spec futura debe cumplir: hexagonal, SOLID, capas separadas
- Definir reglas de testing: solo mocks, 95% coverage, sin acceso a recursos reales
- Hacer que estas reglas sean verificables (CI las impone, no son sugerencias)

**Non-Goals:**
- No define herramientas específicas de CI (eso es implementación del pipeline)
- No define la estructura exacta de `Cargo.toml` de cada crate
- No reemplaza el diseño del cambio fundacional — lo complementa con restricciones

## Decisions

### Gobierno como specs, no como documento aparte
- **Rationale**: Las specs son el mecanismo de contrato del proyecto. Poner gobierno como specs permite que cada cambio futuro declare explícitamente qué reglas cumple y permite verificabilidad en CI. Un README o CONTRIBUTING.md es pasivo y no verificable.

### Dos specs separadas: arquitectura y testing
- **Rationale**: Son concerns ortogonales. Una spec puede cambiar sus reglas de testing sin afectar las de arquitectura, y viceversa. Facilita auditoría y evolución independiente.

### Reglas expresadas como SHALL/MUST (normativas, no sugerencias)
- **Rationale**: "should" y "may" son ambiguos en CI. Las reglas de gobierno deben ser binarias: se cumplen o no. SHALL/MUST permite validación automatizada.

### Cobertura 95% global, no por crate
- **Rationale**: Un crate de tipos puros (domain) puede tener 100% fácilmente; uno de transporte puede requerir más esfuerzo. El 95% global evita castigar crates legítimamente más difíciles de cubrir, siempre que el total cumpla.

## Risks / Trade-offs

- **[Risk] Governance demasiado rígido frena velocidad** → Mitigation: Las reglas se revisan como cualquier spec; si una regla bloquea, se propone un cambio de gobierno
- **[Risk] 95% coverage puede incentivar tests triviales para inflar números** → Mitigation: Code review debe validar que los tests ejercen comportamiento real, no solo pasan líneas
- **[Trade-off] Mocks-only descarta integration tests útiles** → Mitigation: Los integration tests con recursos reales van en un entorno separado (staging/CI de integración), no en la suite de unit tests. Las specs de gobierno aplican a la suite principal