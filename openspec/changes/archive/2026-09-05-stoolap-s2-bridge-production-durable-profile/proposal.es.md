# Propuesta: STOOLAP-S2 — Perfil de Producción durable respaldado por Stoolap

> Compañero en español. Fuente de verdad: `proposal.md` (encabezados 1:1).

## Intención

Producción tiene una única compuerta incondicional: `EntityRuntimeBuilder::validate_persistence`
(`crates/persistent-entity/src/builder.rs:290-305`) exige `is_durable()` tanto al `EventStore<E>` de
la entidad **como** a su `Snapshot`, mediante `require_durably_configured` (`profile.rs:51-63`).
`PersistenceFacade<E>` (`persistence.rs:211-245`) se construye exactamente con esos dos traits: no
existe constructor alternativo ni ruta vía `Repository<A>` hacia la construcción del runtime de
entidad. Por eso `Profile::Production` implica PostgreSQL, y el `StoolapRepository<A>` de S1 queda
fuera de ese camino. Un host futuro (Verimand Bridge, aún no construido) necesita Producción sobre un
archivo embebido. Esta propuesta delimita lo que ego-rs debe ofrecer, no el diseño de Bridge.

## Alcance

### Dentro del alcance

- `EventStore<E>` y `Snapshot` respaldados por Stoolap.
- Durabilidad real: escribir, destruir el runtime, reabrir el mismo archivo, el estado se recupera.
- `Profile::Production` superando `try_build()` de verdad, sin PostgreSQL.
- Modelo operativo: un solo proceso/nodo dueño del archivo (R12 de S1); un solo tenant por proceso
  (`examples/reference-app/src/lib.rs:722-724`).

### Fuera del alcance

- `OperationReservationStore`, `OffsetStore`, `DedupStore`, `ReadSideClaimStore`.
- Cualquier cambio a `Repository<A>`/`StoolapRepository`, o a las compuertas de Producción.
- Reimplementar los effect stores: `StoolapEffectStore` ya satisface sus puertos.
- Stoolap multiproceso o multinodo.
- Envoltorios en memoria que reporten `is_durable() == true`.

Diferido, condicionado a que Bridge adopte idempotencia forzada o proyecciones de lectura durables:
`OperationReservationStore`, persistencia durable del lado de lectura.

## Capacidades

### Capacidades nuevas

- `persistence-stoolap-event-sourcing`: existen `EventStore<E>` y `Snapshot` respaldados por Stoolap,
  sobreviven al reinicio del proceso y permiten que un runtime `Profile::Production` se construya y
  se recupere sin PostgreSQL.

Nueva, no modificada: `openspec/specs/persistence-stoolap-adapter/spec.md` declara en su Propósito
que "no cubre ningún otro store respaldado por Stoolap (`EventStore`, `Snapshot`…)", y R1–R12 tratan
enteramente de `save`/`load`/`delete` de `Repository<A>` y concurrencia optimista.

### Capacidades modificadas

- **Ninguna prevista.** La fase de spec lo confirma; un cambio necesario es una pregunta bloqueante.

## Enfoque

Seguir `StoolapEffectStore` (`crates/effect-store/src/stoolap/mod.rs`), no a S1: ya envuelve la
`Database` síncrona de Stoolap tras traits async mediante `spawn_blocking`, con un clasificador de
errores probado — la forma que requiere la semántica async y append-only de `EventStore`. De S1 se
reutiliza solo lo que encaja: columnas tenant/aggregate/version/payload, el DSN
`file://{path}?sync=full`, `SYSTEMWIDE_SCOPE` + `encode_tenant`. No trasladar el `save` síncrono con
concurrencia optimista de S1 a `EventStore`.

## Áreas afectadas

| Área | Impacto | Descripción |
|------|---------|-------------|
| `crates/persistence-stoolap/` | Modificado | Ambos stores y su esquema (el diseño puede elegir un crate hermano) |
| `crates/persistent-entity/`, `crates/effect-store/` | Intacto | Compuertas y effect store son solo referencia |
| `openspec/specs/persistence-stoolap-event-sourcing/` | Nuevo | Spec de la capacidad |

## Riesgos

| Riesgo | Probabilidad | Mitigación |
|--------|--------------|------------|
| Durabilidad declarada pero no real (modo sync por defecto de Stoolap) | Media | Prueba de reapertura más aserción del modo sync; el KD-2 de S1 documenta esta regresión silenciosa en el repo |
| Copiar el código síncrono de `Repository` de S1 al `EventStore` async | Media | `StoolapEffectStore` es la plantilla; la reutilización de S1 se limita a esquema/DSN/codificación de tenant |
| Presupuesto de revisión por encima de 400 líneas | Alta | Dividir: (1) `Snapshot`, (2) `EventStore`, (3) construcción en Producción + recuperación tras reinicio |

## Plan de reversión

Un solo commit de reversión. Es puramente aditivo: ningún crate existente adquiere una dependencia no
de desarrollo, ninguna ruta del framework cablea los nuevos stores, las compuertas y
`StoolapEffectStore` quedan intactos, y solo se crean las tablas propias del adaptador en su propio
archivo. No hay migración en ninguna dirección; revertir a mitad de camino es igual de seguro.

## Dependencias

- `persistence-api-surface` (ya entregado) — `EventStore<E>` y `Snapshot`, consumidos sin cambios.
- `persistence-stoolap-adapter` (S1) — solo patrones, sin acoplamiento de código.
- `stoolap`, ya fijado en `Cargo.lock`. Ninguna dependencia externa nueva.

## Criterios de éxito

- [ ] Un runtime `Profile::Production` se construye sobre Stoolap sin PostgreSQL en su grafo de dependencias.
- [ ] Se escriben eventos, se destruye el runtime, se reabre el mismo archivo y el estado se recupera idéntico.
- [ ] `validate_persistence` y `require_durably_configured` quedan sin modificar en el diff.
- [ ] Ambos stores reportan `is_durable() == true` porque hacen fsync, no porque un envoltorio lo diga.
- [ ] Ninguna implementación de un store fuera de alcance aparece en el diff.
