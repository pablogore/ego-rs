# Propuesta: STOOLAP-S3 — Operation Reservation Store: señal de durabilidad, implementación Stoolap, gate de Producción

> Compañero en español. Canónico / inglés: `proposal.md` (encabezados 1:1).

## Intent

`OperationReservationStore` (`crates/persistence-api/src/operation/reservation.rs:66-167`) es el
único puerto de `persistence-api` **sin señal de durabilidad**. Todos sus hermanos — `Snapshot`,
`EventStore`, `OffsetStore`, `DedupStore`, `ReadSideClaimStore` — declaran
`fn is_durable(&self) -> bool { false }` más una impl de reenvío `Arc<T>` que es funcional, no
decorativa (por ejemplo `read_side/dedup.rs:33-35`, `59-67`). Al faltar la señal,
`validate_persistence_profile` (`crates/service-sdk/src/runtime/builder.rs:867-872`) no puede
verificar este puerto: `Profile::Production` acepta el deliberadamente volátil
`InMemoryOperationReservationStore` y no obtiene ningún beneficio del genuinamente durable
`PostgresOperationReservationStore`. Es una brecha de contrato en el framework compartido,
independiente de cualquier backend.

STOOLAP-S2 difirió este puerto de forma explícita. S3 lo cierra y agrega la tercera implementación
faltante.

## Scope

### In Scope

- **(a) Corrección del puerto**: `is_durable()` en `OperationReservationStore`, por defecto `false`,
  más la impl de reenvío `Arc<T>`. El mismo patrón de todos los hermanos — no la estructura
  `capabilities()` del crate `effect-store`.
- **(a) Barrido de implementadores**: in-memory se declara no durable; Postgres sobrescribe a `true`;
  se revisa cada mock/fake/doble de prueba (`crates/testkit/src/reservation.rs` reexporta el de memoria).
- **(b)** `StoolapOperationReservationStore`, durable solo una vez que demuestre reserva atómica,
  propiedad, lease, monotonicidad del fencing token y aislamiento por tenant a través de cierre/reapertura.
- **(c)** Gate de Producción cableado igual que `validate_read_side_claim_profile`: falla cerrado
  cuando la composición requiere reservas y el store no es durable. Ningún control existente se
  debilita, ninguna excepción por backend.
- **(d)** Tests de gate para los tres backends (durable aceptado, no durable rechazado) más un test
  de durabilidad por reapertura en Stoolap.

### Out of Scope

- Seguridad multiproceso / multinodo en Stoolap. La evidencia en el repositorio es solo intraproceso;
  `StoolapEffectStore` declara `multi_node_safe: false` (`crates/effect-store/tests/conformance.rs:296-302`).
- Cualquier cambio en la semántica de reservas, la retención o los gates de otros puertos.
- Una segunda forma de expresar durabilidad en este código.

## Capabilities

### New Capabilities

- `persistence-stoolap-operation-reservation`: existe un reservation store respaldado por Stoolap,
  preserva cada invariante de reserva y sobrevive a cierre/reapertura.

### Modified Capabilities

- `persistence-api-surface`: el puerto de reservas gana la señal de durabilidad y su reenvío `Arc`.
- `idempotent-command-processing`: el reservation store de PostgreSQL se reporta durable.
- `persistence-memory-adapter`: el reservation store en memoria se reporta no durable.
- `production-composition-hardening`: un gate del reservation store bajo `Profile::Production`.

## Approach

Copiar textualmente el patrón de los puertos hermanos para (a). Para (b), seguir `StoolapEffectStore`
(`crates/effect-store/src/stoolap/mod.rs`): `spawn_blocking` sobre la `Database` síncrona, `open()`
que falla cerrado si no hay `sync=full`, `INSERT ... ON CONFLICT DO NOTHING` + re-`SELECT` para
clasificar resultados, y el compare-and-swap `UPDATE ... WHERE version = $N` ya usado en
`repository.rs:23-24` / `snapshot.rs:40-41` para el fencing. Para (c), reutilizar
`require_durably_configured`.

## Affected Areas

| Área | Impacto | Porción |
|------|---------|---------|
| `crates/persistence-api/src/operation/reservation.rs` | Modificado | (a) |
| `crates/persistence-memory/src/operation/reservation.rs`, `crates/persistence/src/postgres/reservation.rs`, `crates/testkit/src/reservation.rs` | Modificado | (a) |
| `crates/persistence-stoolap/` | Nuevo | (b) |
| `crates/service-sdk/src/runtime/builder.rs` | Modificado | (c) |
| tests en los crates anteriores | Nuevo | (d) |

## Risks

| Riesgo | Probabilidad | Mitigación |
|--------|--------------|------------|
| Durabilidad declarada pero no real (Stoolap no hace fsync por defecto) | Media | Test de reapertura más una aserción de modo de sincronización antes de activar el flag |
| Un implementador externo hereda `false` en silencio y rompe un build de Producción | Media | El `false` por defecto es la respuesta honesta; el mensaje de rechazo nombra la corrección, según "Rejections Are Actionable" |
| Omitir el reenvío `Arc` degrada un store durable al valor por defecto | Media | La impl de reenvío se entrega junto al método del trait; el test de gate envuelve en `Arc` |
| Presupuesto de revisión sobre 400 líneas | Alta | Dividir por (a) / (b) / (c)+(d) |

## Rollback Plan

Revertir por porción, de la más nueva a la más antigua. (c) es la única pieza que cambia
comportamiento y se revierte sola. (b) es puramente aditiva. (a) se revierte como eliminación de
firma; el valor por defecto `false` significa que ningún implementador externo queda roto mientras
está vigente.

## Dependencies

- `persistence-api-surface`, `persistence-stoolap-adapter` (S1), `persistence-stoolap-event-sourcing`
  (S2) — ya entregados. `stoolap` ya está fijado en `Cargo.lock`. Sin dependencias externas nuevas.

## Success Criteria

- [ ] `OperationReservationStore::is_durable()` coincide exactamente con los puertos hermanos, reenvío `Arc` incluido.
- [ ] In-memory reporta `false`, Postgres y Stoolap reportan `true`; ningún implementador queda sin revisar.
- [ ] Stoolap: reserva escrita, base de datos cerrada, mismo archivo reabierto, propiedad + fencing token + alcance de tenant intactos.
- [ ] `Profile::Production` rechaza un reservation store no durable y acepta ambos durables.
- [ ] Ningún control de gate preexistente se debilita, y ninguna afirmación sobre Stoolap excede la concurrencia intraproceso.
