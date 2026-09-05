# Diseño: STOOLAP-S2 — Perfil de Producción Durable Respaldado por Stoolap

> Compañero en español. Canónico / inglés: `design.md` (encabezados 1:1).

## Enfoque Técnico

Dos almacenes nuevos dentro del crate existente `ego-persistence-stoolap`. `Snapshot`
(`persistence-api/src/persistence/snapshot.rs:14`) es **síncrono**, por lo que no requiere nada que S1
no tenga ya. `EventStore<E>` (`event_store.rs:47`) es `#[async_trait]`, así que solo él necesita
`spawn_blocking` — copiado en forma de `StoolapEffectStore::run_blocking`
(`effect-store/src/stoolap/mod.rs:227`). No se toca ninguna compuerta, ni la fachada, ni el código de
`Repository<A>`.

## Decisiones de Arquitectura

### AD-1: Ubicación del crate — mismo crate, feature `event-sourcing`

| Opción | Compensación | Decisión |
|---|---|---|
| Crate hermano `ego-persistence-stoolap-event-sourcing` | Mantiene S1 libre de tokio, pero `dsn_for`/`encode_tenant`/`is_write_conflict`/`internal_err` son funciones privadas de `repository.rs` — habría que copiarlas o exportarlas | Rechazada: duplica justo lo que AD-2 quiere compartir |
| Mismo crate, dependencias `tokio` + `async-trait` incondicionales | Diff mínimo, pero todo consumidor de `Repository<A>` gana un runtime asíncrono, contradiciendo la afirmación de rollback de la propuesta ("ningún crate existente gana una dependencia no-dev") | Rechazada |
| **Mismo crate, módulo `event_sourcing` detrás de `event-sourcing = ["dep:tokio", "dep:async-trait"]`** | Dos líneas de Cargo; helpers compartidos dentro del crate; consumidores síncronos intactos | **Elegida** — refleja el patrón de dependencias opcionales del propio `ego-effect-store` (`effect-store/Cargo.toml:46-50`), que es un crate único con backends por feature, no una separación sync/async |

`Snapshot` queda **fuera** de la feature: no agrega ninguna dependencia.

### AD-2: Frontera de reutilización

| Origen | Se reutiliza | No se reutiliza |
|---|---|---|
| `repository.rs` de S1 | `dsn_for` (`file://{p}?sync=full`), `SYSTEMWIDE_SCOPE`, `encode_tenant`, `internal_err`, `is_write_conflict` — promovidos de privados a `pub(crate)` en un módulo nuevo `stoolap_common`; la forma de la tabla `aggregates` (`tenant_id/aggregate_id/version/payload` + `UNIQUE(...)`) reutilizada literalmente como `snapshots` | El cuerpo síncrono lectura-verificación-escritura de `save()` |
| `StoolapEffectStore` | `run_blocking` (clonar `Database`, `spawn_blocking` — **no** `block_in_place`, que entra en pánico en runtimes current-thread); reglas del dialecto: `UNIQUE` en vez de `PRIMARY KEY` compuesta, payloads TEXT (no hay BYTEA), nada de `DELETE ... IN (SELECT ...)` | `backend_err` — S2 devuelve `PersistenceError`, así que clasifica `is_write_conflict` de S1 |

### AD-3: La durabilidad se verifica, no se afirma

`StoolapEffectStore::open` construye `file://{path}` **sin** `sync=full` y aun así reporta
`durable: true` — exactamente el defecto que este diseño no debe repetir. Regla:

1. Ambos almacenes abren únicamente a través del `dsn_for()` compartido.
2. `open()` abre a través del `dsn_for()` compartido (`sync=full`) y luego relee `db.dsn()`,
   devolviendo `PersistenceError::Internal` si no lleva `sync=full` — una segunda comprobación
   defensiva. Verificado durante la implementación (`snapshot.rs`,
   `open_refuses_a_path_already_locked_by_a_non_durable_engine`): el registro global de proceso de
   Stoolap comparte un motor vivo solo para un DSN *idéntico*
   (`effect-store/src/stoolap/mod.rs:170-173`) — un DSN distinto para la misma ruta, como un motor ya
   abierto con sync más débil, nunca se devuelve. En cambio, `Database::open()` falla directamente con
   `stoolap::Error::DatabaseLocked` (el bloqueo de archivo en disco ya está tomado), capturado por el
   mismo `map_err(internal_err)` que cualquier otro fallo de apertura, antes de que se ejecute la
   comprobación de `sync=full`. Ambos modos de fallo cierran de la misma manera.
3. `is_durable() -> true` queda entonces respaldado por un invariante de construcción, no por
   presencia — la propiedad que exige `require_durably_configured` (`profile.rs:44-50`).
4. `append` y el `commit` de la unidad de trabajo terminan cada uno en exactamente un `tx.commit()`.
   No puede existir una ruta de commit diferido o por lotes, o `sync=full` deja de significar "durable
   cuando la llamada retorna".

### AD-4: Recuperación tras reinicio por la ruta de composición real

Plantilla: el test `a_committed_save_survives_close_and_reopen` de S1 (`repository.rs:344-367`) — un
ámbito interno escribe y suelta, el externo reabre la misma ruta `TempDir`. Adaptado a event sourcing:

```
{ // fase 1
  EntityRuntimeBuilder::new().profile(Profile::Production)
    .with_event_store(Arc::new(StoolapEventStore::open(path).await?))
    .with_snapshot_store(Arc::new(Mutex::new(StoolapSnapshotStore::open(path)?)))
    .with_snapshot_strategy(/* dispara por debajo del número de eventos */)
    .try_build()?                       // la compuerta real, sin atajos
  // enviar comandos -> eventos que cruzan el umbral de snapshot
}                                       // el drop libera el motor
// fase 2: misma cadena del builder, misma ruta -> estado y versión coinciden
```

Más un control negativo en el mismo archivo: `Profile::Production` + almacenes en memoria sigue
rechazando.

## Flujo de Datos

    EntityRuntimeBuilder(Production) ──validate_persistence──> is_durable()==true (invariante AD-3)
              │
        PersistenceFacade ──> StoolapEventStore ──spawn_blocking──> Database (sync=full)
              │                                                          │
              └──────────> StoolapSnapshotStore ──(síncrono, directo)────┘  un archivo

## Cambios de Archivos

| Archivo | Acción | Descripción |
|---|---|---|
| `crates/persistence-stoolap/src/persistence/stoolap_common.rs` | Crear | DSN, codificación de tenant y clasificación de errores promovidos a `pub(crate)` |
| `crates/persistence-stoolap/src/persistence/repository.rs` | Modificar | Importa los helpers promovidos; comportamiento sin cambios |
| `crates/persistence-stoolap/src/persistence/snapshot.rs` | Crear | Rebanada 1 — `Snapshot` síncrono + tabla `snapshots` + guardia de DSN |
| `crates/persistence-stoolap/src/event_sourcing/event_store.rs` | Crear | Rebanada 2 — `EventStore<E>`, unidad de trabajo, recibos |
| `crates/persistence-stoolap/Cargo.toml` | Modificar | `tokio`/`async-trait` opcionales, feature `event-sourcing`, dev-dep `ego-persistent-entity` |
| `crates/persistence-stoolap/tests/production_restart_recovery.rs` | Crear | Rebanada 3 — recuperación tras reinicio + control negativo |

Sin ciclo: `persistent-entity` no depende de `persistence-stoolap`.

## Estrategia de Pruebas

| Rebanada | Capa | Qué | Cómo |
|---|---|---|---|
| 1 | Unitaria | El DSN lleva `sync=full`; se rechaza un motor sin `sync=full`; ida y vuelta del snapshot; aislamiento tenant vs systemwide | Colocadas, `TempDir`, `db_test_guard()` de S1 |
| 1 | Unitaria | Un sync de WAL fallido aflora como error, nunca como éxito silencioso | `stoolap::test_failpoints` (`WAL_WRITE_FAIL`), como hace S1 |
| 2 | Unitaria | append/load/list, conflicto de concurrencia optimista, conflicto de recibo, una UoW soltada no deja nada | Colocadas, `#[tokio::test]` (current-thread) |
| 3 | Integración | Build de producción, escribir, soltar, reabrir, estado+versión idénticos; los almacenes volátiles siguen rechazados | `tests/production_restart_recovery.rs` |

Cada rebanada deja el workspace en verde por sí sola: la 1 no agrega dependencias, la 2 agrega la
feature, la 3 es solo de pruebas.

## Matriz de Amenazas

N/A — no hay enrutamiento, shell, subprocesos, automatización de VCS/PR, clasificación de archivos
ejecutables ni frontera de integración de procesos.

## Migración / Despliegue

Sin migración. Puramente aditivo; `CREATE TABLE IF NOT EXISTS` en el archivo propio del adaptador.

## Preguntas Abiertas

- [ ] Si `sync=full` hace fsync por commit o por intervalo es un hecho interno de Stoolap que este
      diseño afirma a partir de la elección de DSN de S1, no de leer Stoolap. El test de failpoint de
      la rebanada 1 es la compuerta: si un sync de WAL suprimido no aflora como error, AD-3 queda sin
      probar y la rebanada 2 debe detenerse.
- [ ] Reabrir tras un drop prueba la recuperación de cierre limpio, no la durabilidad ante un kill -9.
      Queda fuera de alcance aquí; nómbrese en el requisito de durabilidad de la spec en lugar de
      insinuar más.
