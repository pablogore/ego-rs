## Why

Sin estándares explícitos, cada cambio implementa su propio estilo de arquitectura y testing. Esto genera deriva técnica desde el día uno. Establecer gobierno ahora —cuando solo existe un cambio fundacional— evita deuda estructural antes de que se acumule. Todo cambio futuro debe cumplir estas reglas: arquitectura hexagonal, SOLID, tests con mocks al 95% de coverage y cero acceso a recursos reales.

## What Changes

- Se definen reglas mandatorias de arquitectura: hexagonal (puertos y adaptadores), principios SOLID, clean architecture con separación domain/application/infrastructure/transport
- Se define la estrategia de testing: solo tests unitarios con mocks, 95% de coverage mínimo, nunca acceder a Kafka, bases de datos, endpoints externos u otros recursos reales en tests
- Se establece el gobierno como spec de proyecto que toda spec futura debe respetar

## Capabilities

### New Capabilities
- `architecture-governance`: Estándares mandatorios de arquitectura: hexagonal, SOLID, separación de capas, dirección de dependencias
- `testing-governance`: Estrategia mandatoria de testing: mock-first, 95% coverage, cero recursos reales en tests

### Modified Capabilities
<!-- None — governance is a new concern, no existing specs to modify. -->

## Impact

- **Proceso**: Toda spec futura debe validarse contra estas reglas de gobierno antes de ser aceptada
- **CI**: El pipeline debe fallar si coverage < 95% o si algún test accede a recursos reales (red, disco, I/O externo)
- **Código**: No impacta código existente directamente; establece el contrato para todo código futuro
- **Dependencias**: mockall, cargo-tarpaulin ya están considerados en el cambio fundacional; aquí se formaliza su uso obligatorio