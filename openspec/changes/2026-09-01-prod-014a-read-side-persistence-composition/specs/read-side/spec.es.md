# Delta para read-side

> Documento acompañante para revisión. La fuente de verdad canónica es
> `spec.md` (identificadores 1:1). Spec base de la capacidad:
> `openspec/specs/read-side/spec.md` (CORE-026). Este delta se aplica
> sobre la sección "Non-Goals" existente de ese spec, en particular la
> viñeta "Constructing a dedup store, an offset store, or a tag-discovery
> mechanism is out of scope."

Alcance: PROD-014A. Esto es una clarificación de límite (D-5), no una
renegociación de CORE-026. Dos ejes: que el framework construya o
defaultee los stores del read-side sigue siendo un non-goal, sin
cambios; que la raíz de composición acepte, clasifique y valide un par
construido por el host es nuevo, dentro de alcance. La superficie de
registro y el mecanismo de rechazo bajo Production están especificados
por los deltas de `application-composition` y
`production-composition-hardening` — este delta solo establece que el
límite lo permite.

## Requisitos AGREGADOS

### Requisito: La Aceptación en la Raíz de Composición de un Par de Progreso Durable Construido por el Host Está en Alcance; La Construcción por el Framework Sigue Fuera de Alcance

El par de progreso durable de una proyección — su `OffsetStore` y su
`DedupStore` — PUEDE componerse en la raíz de composición: aceptado,
clasificado por durabilidad, y rechazado allí bajo `Profile::Production`
cuando no es durable. Esto es ortogonal a, y no revierte, el non-goal
existente de CORE-026 de que el framework construya o defaultee estos
stores internamente — ese non-goal sigue completamente vigente. La raíz
de composición nunca construye internamente un `OffsetStore`, un
`DedupStore`, ni un mecanismo de descubrimiento de tags en nombre de la
aplicación; solo acepta, clasifica y valida un par que la aplicación ya
construyó.

#### Escenario: La raíz de composición clasifica y valida sin construir

- DADO una aplicación que ya construyó su propio par
  `OffsetStore`/`DedupStore`
- CUANDO lo registra en la raíz de composición
- ENTONCES la raíz de composición clasifica y valida la durabilidad del
  par sin construir ninguno de los dos stores

#### Escenario: Una aplicación que no registra nada no se ve afectada

- DADO una aplicación que nunca registra un par de progreso durable en la
  raíz de composición
- CUANDO compone su wiring de read-side exactamente como antes de este
  cambio
- ENTONCES esta capacidad no requiere ni realiza nada de ese wiring, sin
  cambios respecto a antes

#### Escenario: El rechazo nunca llega al motor del scheduler

- DADO un par registrado rechazado bajo `Profile::Production`
- CUANDO ocurre ese rechazo
- ENTONCES ocurre en la raíz de composición, nunca dentro de
  `ProjectionSpec::new()`, `TagSchedulerImpl::spawn()`, ni el primer lote
  de sondeo

## Non-Goals

- Introducir `Profile` en `ProjectionSpec`, `TagSchedulerImpl`,
  `ReadSideSession`, o `ReadSideRunner` sigue fuera de alcance. Sin
  cambios en la semántica de polling, dedup, offset u orden. El rechazo
  que este delta permite ocurre solo en la raíz de composición.
- Que el framework construya o defaultee `OffsetStore`, `DedupStore`, o
  un mecanismo de descubrimiento de tags sigue completamente fuera de
  alcance — es el non-goal existente de CORE-026 anterior, sin cambios y
  no renegociado por este delta.
- La durabilidad de `ReadSideStore` (la fuente de eventos que sondea una
  proyección) no está gobernada por este delta — es una vista de lectura
  del event store, no estado de resume.
