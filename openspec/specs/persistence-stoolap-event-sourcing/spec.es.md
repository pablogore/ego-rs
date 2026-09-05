# Spec: `persistence-stoolap-event-sourcing` (Capacidad Nueva)

> Compañero en español. Canónico / inglés: `spec.md` (IDs de requisito y escenarios 1:1).

## Propósito

El contrato observable de que existen un `EventStore<E>` y un `Snapshot` respaldados por Stoolap,
que `Profile::Production` construye y pasa `validate_persistence` sobre ellos sin ningún PostgreSQL
involucrado, que el estado confirmado antes del apagado es genuinamente recuperable desde el mismo
archivo tras destruir y reabrir el runtime, y que `is_durable()` refleja el comportamiento de sync
real en vez de una bandera fija. No cubre `OperationReservationStore`, `OffsetStore`, `DedupStore`,
`ReadSideClaimStore`, ningún cambio a `Repository<A>`/`StoolapRepository`, ni el comportamiento
existente de `StoolapEffectStore` — todo eso queda fuera de alcance, sin afectar.

## Requisitos

### Requirement: Production Builds Without PostgreSQL

Un `EntityRuntimeBuilder` configurado con `Profile::Production`, un `EventStore<E>` respaldado por
Stoolap y un `Snapshot` respaldado por Stoolap DEBE tener éxito en `build()`/`try_build()` y pasar
la compuerta de durabilidad de `validate_persistence`, sin ninguna conexión, driver o dependencia de
PostgreSQL involucrada en producir ese éxito.

#### Scenario: Production build succeeds on Stoolap alone
- DADO `Profile::Production` con un `EventStore<E>` y un `Snapshot` respaldados por Stoolap
  registrados, y ninguna configuración de PostgreSQL presente
- CUANDO corre `try_build()`
- ENTONCES tiene éxito, y no se abre ninguna conexión a PostgreSQL en ningún momento

### Requirement: Committed State Survives Runtime Destruction and File Reopen

Una vez que los eventos de una entidad (y cualquier snapshot) quedan confirmados en un archivo
Stoolap, ese estado DEBE ser recuperable, sin cambios, por un runtime nuevo que reabre el mismo
archivo después de que el runtime original y sus recursos a nivel de proceso quedan completamente
liberados.

#### Scenario: Write, drop, reopen, state matches
- DADO un runtime `Profile::Production` respaldado por un archivo Stoolap, con los comandos de una
  entidad aplicados y confirmados
- CUANDO el runtime (y sus handles de `EventStore`/`Snapshot`) se libera, y luego un runtime nuevo
  abre la misma ruta de archivo
- ENTONCES recuperar la misma entidad produce un estado idéntico al confirmado antes de la
  liberación

### Requirement: Durability Claims Reflect Real Sync Behavior

`is_durable() == true` en el `EventStore<E>` y el `Snapshot` respaldados por Stoolap DEBE
corresponder a una configuración de sync durable genuina del almacén subyacente, no a un valor de
retorno fijo. Una instancia de almacén que no está realmente configurada para sync durable NO DEBE
reportar `is_durable() == true`.

#### Scenario: A durably-configured store's claim is truthful
- DADO un `EventStore<E>` respaldado por Stoolap abierto en su configuración de sync durable
- CUANDO se llama a `is_durable()`
- ENTONCES devuelve `true`, y una escritura confirmada bajo esa configuración sobrevive el
  escenario de reapertura de arriba

#### Scenario: is_durable is not a fixed constant independent of configuration
- DADO dos instancias de almacén respaldadas por Stoolap que difieren solo en su configuración de
  sync
- CUANDO se llama a `is_durable()` en cada una
- ENTONCES los resultados reflejan la configuración real de cada instancia, no un valor idéntico
  fijo

### Requirement: File Ownership Is Single-Process, Single-Node Only

El `EventStore<E>` y el `Snapshot` respaldados por Stoolap DEBEN garantizar comportamiento
concurrente correcto solo entre llamadores dentro de un proceso propietario en un nodo, igualando
la no-garantía ya establecida para `persistence-stoolap-adapter`. No se hace ninguna garantía de
acceso concurrente multi-proceso ni multi-nodo.

#### Scenario: Multi-process access is an explicit non-guarantee
- DADOS dos procesos de sistema operativo separados que abren el mismo archivo Stoolap
- CUANDO ambos acceden concurrentemente
- ENTONCES esta capacidad no documenta ninguna garantía de comportamiento correcto o seguro para
  ese caso

### Requirement: Tenant Scoping Is Honored Correctly

Las implementaciones respaldadas por Stoolap DEBEN enhebrar correctamente el parámetro existente
`tenant_id: Option<&str>` de `EventStore` y `Snapshot`: los eventos/snapshots de un tenant nombrado
NO DEBEN ser visibles bajo, ni confundirse con, un tenant nombrado diferente o el alcance systemwide
(`None`), bajo el modelo de un-solo-tenant-por-proceso que `Profile::Production` ya exige.

#### Scenario: A tenant's events are isolated from another tenant sharing the same aggregate identity
- DADA la misma identidad de agregado escrita independientemente bajo dos valores de `tenant_id`
  distintos
- CUANDO se carga cada uno
- ENTONCES cada uno devuelve solo los eventos de su propio tenant, sin visibilidad cruzada entre
  tenants

## No-Objetivos

- `OperationReservationStore`, `OffsetStore`, `DedupStore`, `ReadSideClaimStore` — ningún requisito
  de esta spec implica que estos ganen una implementación respaldada por Stoolap.
- Cualquier cambio a `Repository<A>` o `StoolapRepository` (S1) — sin tocar.
- El comportamiento de `StoolapEffectStore` — ya satisfecho, sin verse afectado por esta capacidad.
- Compartir un mismo archivo Stoolap entre múltiples tenants por proceso — sigue sin soportarse,
  según la restricción existente de un-solo-tenant de `Profile::Production`.
