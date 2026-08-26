# Delta para Persistent Entity

## Requisitos AGREGADOS

### Requisito: EntityRuntimeBuilder Condiciona el Fallback en Memoria por Profile

`EntityRuntimeBuilder` DEBE aceptar una declaración de `Profile`
(`Profile::Dev` por defecto, `Profile::Production`). Los dos fallbacks
`unwrap_or_else` de `EntityRuntimeBuilder::build()` hacia
`InMemoryEventStore` e `InMemorySnapshotStore` DEBEN ejecutarse solo bajo
`Profile::Dev`. Bajo `Profile::Production`, un event store o snapshot
store faltante DEBE rechazar el arranque con un error local que nombra la
capacidad y la llamada de configuración exacta (`.with_event_store()` /
`.with_snapshot_store()`) que la corrige, en lugar de caer silenciosamente
al fallback. Dado que `persistent-entity` no depende de `service-sdk`,
este tipo de error DEBE definirse localmente en `persistent-entity` y
cruzar el límite de capas de la misma forma en que
`RuntimeError::OperationReservationStoreNotRegistered` ya lo hace una capa
más arriba.

#### Escenario: Production sin event store rechaza en lugar de caer al fallback
- DADO `Profile::Production` y ninguna llamada a `.with_event_store()`
- CUANDO corre `EntityRuntimeBuilder::build()`
- ENTONCES rechaza con un error que nombra el event store y
  `.with_event_store()`; `InMemoryEventStore` nunca se construye

#### Escenario: Production sin snapshot store rechaza en lugar de caer al fallback
- DADO `Profile::Production` y ninguna llamada a `.with_snapshot_store()`
- CUANDO corre `EntityRuntimeBuilder::build()`
- ENTONCES rechaza con un error que nombra el snapshot store y
  `.with_snapshot_store()`; `InMemorySnapshotStore` nunca se construye

#### Escenario: El profile Dev preserva el fallback silencioso actual sin cambios
- DADO `Profile::Dev` (el default) y ninguno de los dos stores configurado
- CUANDO corre `EntityRuntimeBuilder::build()`
- ENTONCES tiene éxito sobre `InMemoryEventStore` e `InMemorySnapshotStore`,
  byte a byte como antes de este cambio

### Requisito: La Configuración Parcial de Event/Snapshot Bajo Production Ya Está Cubierta Por las Puertas de Cada Capacidad

`EntityRuntimeBuilder::build()` no necesita un chequeo de configuración
parcial separado. Bajo `Profile::Production`, si exactamente uno de
`{event_store, snapshot_store}` fue configurado explícitamente y el otro
no, el fallback condicionado por profile de arriba ya lo rechaza — la
puerta propia de la capacidad faltante se dispara porque, de hecho, falta.
Bajo `Profile::Dev`, la configuración parcial permanece válida, sin
cambios respecto del comportamiento actual; esto no es una exención nueva
sino el mismo comportamiento del que ya depende cada sitio de llamada
parcial existente, incluyendo 14 cadenas de test bajo
`crates/persistent-entity/tests/` y
`examples/reference-app/src/lib.rs:502` (`design.md` §Evidence
Corrections, EC-1).

#### Escenario: Un store configurado, el otro faltante, rechazado bajo Production vía su propia puerta
- DADO `Profile::Production` con `.with_event_store()` llamado y
  `.with_snapshot_store()` nunca llamado
- CUANDO corre `EntityRuntimeBuilder::build()`
- ENTONCES rechaza vía el fallback condicionado por profile del snapshot
  store de arriba (nombrando el snapshot store y la corrección), no vía un
  chequeo de configuración parcial separado

#### Escenario: Un store configurado, el otro faltante, permanece válido bajo Dev
- DADO `Profile::Dev` (el default) con `.with_event_store()` llamado y
  `.with_snapshot_store()` nunca llamado
- CUANDO corre `EntityRuntimeBuilder::build()`
- ENTONCES tiene éxito, cayendo a `InMemorySnapshotStore` para la
  capacidad sin configurar, sin cambios respecto de hoy

### Requisito: Los Sitios de Llamada Existentes de EntityRuntimeBuilder No Se Afectan

Los 67 sitios de llamada existentes a `EntityRuntimeBuilder::new()`,
ninguno de los cuales declara `Profile::Production`, DEBEN seguir
compilando y pasando sin modificación después de que este cambio se envía.

#### Escenario: Un sitio de llamada sin modificar sigue compilando y pasando
- DADO cualquiera de los 67 sitios de llamada existentes, ninguno declara
  `Profile::Production`
- CUANDO se reconstruye el workspace tras este cambio
- ENTONCES compila y sus tests pasan sin ninguna modificación
