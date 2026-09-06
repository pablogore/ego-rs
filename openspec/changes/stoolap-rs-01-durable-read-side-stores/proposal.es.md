# Propuesta: STOOLAP-RS-01 — Stores durables de read-side sobre Stoolap

> Compañero en español. Canónico / inglés: `proposal.md` (encabezados 1:1).

## Intent

Tres puertos de read-side — `OffsetStore`, `DedupStore`, `ReadSideClaimStore` — tienen exactamente
una implementación durable cada uno, y las tres son PostgreSQL. `Profile::Production` verifica los
tres (`validate_read_side_progress_profile` / `validate_read_side_claim_profile`,
`crates/service-sdk/src/runtime/builder.rs:905-945`), así que una composición de producción basada
solo en Stoolap no puede ejecutar ninguna proyección. S1 (repository), S2 (event sourcing) y S3
(operation reservation) cerraron todos los demás puertos durables; el trío de read-side es el último
hueco del perfil durable de Stoolap.

Es una capacidad general del framework. Ningún nombre, contrato ni forma de proyección específica de
un producto entra en este cambio.

## Scope

### In Scope

- `OffsetStore`, `DedupStore` y `ReadSideClaimStore` respaldados por Stoolap, detrás de una nueva
  feature `read-side` en el crate existente `crates/persistence-stoolap`.
- Durabilidad real sobre archivo: valor escrito, base de datos cerrada, mismo archivo reabierto,
  valor intacto. `is_durable()` devuelve `true` solo una vez que eso queda demostrado por store.
- Aislamiento exactamente según la clave de cada puerto — offsets por
  `(projection_id, tag, tenant)`; dedup por `(projection_id, tag, event_id)`, **sin** parámetro de
  tenant (intencional en ese puerto, no es una brecha); claims por
  `ClaimId { projection_id, tag, tenant }`.
- Corrección de claims bajo concurrencia intraproceso real: exclusión, takeover tras expiración del
  lease, rechazo de owner obsoleto, avance del fencing token. `lease_until` lo sigue calculando el
  llamador vía el `Clock` inyectado; `try_claim` devolviendo `Ok(None)` es un rechazo, no un error.
- Un test de composición **real** con `Profile::Production` sobre una base Stoolap en disco, más un
  control negativo que demuestre que un store volátil sigue siendo rechazado. Hoy ese gate solo tiene
  cobertura con stubs (`builder.rs:3950-4010`), a diferencia de Postgres
  (`integration-tests/tests/infrastructure/read_side_progress_postgres.rs`).
- Tests de conformidad para los tres stores.

### Out of Scope

- Todos los demás puertos: `EventStore`, `Snapshot`, `Repository`, `EffectStateStore`,
  `EffectDedupStore`, `OperationReservationStore` — ya entregados. El último se reutiliza únicamente
  como plantilla del patrón de concurrencia.
- Cualquier cambio en PostgreSQL, y cualquier cambio en los tres contratos de trait. No se encontró
  evidencia de que ninguno sea defectuoso.
- Stoolap multiproceso, coordinación multinodo/distribuida, elección de líder en Kubernetes,
  `LISTEN`/`NOTIFY`, brokers, buses de eventos. Un solo proceso ego-rs es dueño del archivo. La
  concurrencia **dentro** de ese proceso (múltiples tasks, workers, sesiones de read-side) sí está en
  alcance; la coordinación entre procesos no lo está, y nada de lo entregado aquí puede afirmarla.
- Poda, TTL o retención de dedup — un Non-Goal explícito de la spec de read-side existente.
- Monotonicidad o compare-and-swap en `write_offset`. Ese puerto es last-write-wins por contrato.

## Capabilities

### New Capabilities

- `persistence-stoolap-read-side`: stores de offset, dedup y read-side claim respaldados por Stoolap
  que sobreviven a cierre/reapertura, aíslan según las claves reales de cada puerto, se mantienen
  correctos bajo concurrencia intraproceso y satisfacen los gates de Producción existentes.

### Modified Capabilities

- Ninguna. Los gates de read-side de `Profile::Production` ya existen y no se debilitan, relajan ni
  vuelven a especificar. Este cambio agrega un backend que los satisface.

## Approach

Seguir `StoolapOperationReservationStore` (`crates/persistence-stoolap/src/operation/reservation.rs`)
como plantilla — **no** los stores de read-side de Postgres. `try_claim` porta su CAS de dos
sentencias: `INSERT ... ON CONFLICT DO NOTHING` y luego un `UPDATE` condicional que reverifica la
fila viva. Postgres usa un `INSERT ... ON CONFLICT DO UPDATE ... WHERE ... RETURNING` de una sola
sentencia; nada en este repositorio ha demostrado nunca que esa forma funcione en Stoolap 0.4,
mientras que la forma de dos sentencias ya está probada bajo estrés para concurrencia intraproceso
(`crates/persistence-stoolap/tests/reservation_conformance.rs:192-240`). Offsets y dedup son upserts
simples.

El puente asíncrono reutiliza el `run_blocking()` → `tokio::task::spawn_blocking` por store del
crate, nunca `block_in_place`, en consistencia con todos los stores Stoolap existentes. La feature
`read-side` se compone de `tokio`, `async-trait`, `chrono` y `ego-domain`, ya opcionales en ese
manifiesto — cero dependencias transitivas nuevas, y ningún crate nuevo.

## Affected Areas

| Área | Impacto | Descripción |
|------|---------|-------------|
| `crates/persistence-stoolap/Cargo.toml` | Modificado | Nueva feature `read-side`; sin dependencias nuevas |
| `crates/persistence-stoolap/src/read_side/` | Nuevo | Los tres stores más el esquema |
| `crates/persistence-stoolap/tests/` | Nuevo | Tests de conformidad y de durabilidad por reapertura |
| Test de composición con perfil de Producción (ubicación del crate según el diseño) | Nuevo | Composición real con Stoolap + control negativo volátil |
| `crates/persistence-api/src/read_side/` | Sin cambios | Contratos consumidos tal cual |

## Risks

| Riesgo | Probabilidad | Mitigación |
|--------|--------------|------------|
| Stoolap 0.4 rechaza o ejecuta mal el SQL del CAS de claim | Media | Portar el patrón de dos sentencias ya probado, no el de una sentencia de Postgres; el test de conformidad es el control |
| Durabilidad declarada pero no real (Stoolap no hace fsync por defecto) | Media | Test de reapertura más el chequeo `open()` fail-closed con `sync=full` ya existente en el crate antes de que `is_durable()` devuelva `true` |
| El gate de Producción queda verificado solo con stubs y una composición real igual se rompe | Media | El test de composición real está en alcance, no es pulido opcional; el control negativo se entrega con él |
| Expansión de alcance hacia afirmaciones multiproceso/multinodo | Media | Non-goal declarado en el texto de la spec y en el doc de módulo de cada store |
| Presupuesto de revisión sobre 400 líneas | Alta | Dividir por store: offset+dedup, claim, test del gate de composición |

## Open Questions

1. **¿Arnés de conformidad compartido o tests solo en Stoolap?** `ego-testkit` tiene arneses para
   reservation, carrier, event store y repository, pero ninguno para `OffsetStore`/`DedupStore`/
   `ReadSideClaimStore`. Agregar tres arneses compartidos beneficia a futuros backends pero amplía el
   diff. Lo decide la fase de diseño.
2. **¿Dónde vive el test de composición real?** `integration-tests/` refleja el precedente de
   Postgres, pero Stoolap no necesita contenedor — un directorio `tempfile` en
   `persistence-stoolap/tests/` puede bastar. Lo decide la fase de diseño.
3. **¿`real-infrastructure-verification` necesita un delta?** Esa capacidad hoy no nombra ningún
   backend Stoolap. Si un requisito de composición real corresponde allí y no a la capacidad nueva,
   la fase de spec debe agregar un delta.

## Rollback Plan

Revertir por porción, de la más nueva a la más antigua. El cambio completo es aditivo y está detrás
de una feature: con la feature `read-side` apagada, `cargo build` y `cargo test --workspace` se
comportan exactamente igual que antes. No se modifica ningún store, gate ni trait existente, así que
una reversión completa elimina archivos y una entrada de feature, y no toca nada ya en uso
productivo.

## Dependencies

- `persistence-stoolap-adapter` (S1), `persistence-stoolap-event-sourcing` (S2),
  `persistence-stoolap-operation-reservation` (S3) — todos entregados.
- `stoolap` 0.4 ya está fijado en `Cargo.lock`. Sin dependencias externas nuevas.

## Success Criteria

- [ ] Los tres stores existen detrás de `read-side` en `persistence-stoolap`; `cargo check --workspace` con la feature apagada no cambia.
- [ ] Cada store: valor escrito, base de datos cerrada, mismo archivo reabierto, valor intacto — y solo entonces `is_durable()` devuelve `true`.
- [ ] Aislamiento demostrado según las claves reales de cada puerto, incluida la ausencia deliberada de parámetro de tenant en dedup.
- [ ] `try_claim` concurrente desde múltiples tasks en un proceso produce exactamente un holder; takeover, rechazo de owner obsoleto y avance del fencing token verificados.
- [ ] Una composición real con `Profile::Production` sobre una base Stoolap en disco se construye correctamente, y un store volátil en la misma composición es rechazado.
- [ ] Ningún artefacto entregado afirma coordinación multiproceso, multinodo o distribuida.
