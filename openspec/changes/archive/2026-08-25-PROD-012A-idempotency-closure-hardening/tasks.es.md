# Tareas: PROD-012A — Endurecimiento de Cierre de Idempotencia

## Cerradas

- [x] **Fix 1 — Bypass estructural cerrado.** Nuevo archivo
  `crates/service-sdk/tests/idempotent_marker_lint.rs`, que refleja el
  mecanismo existente de `tenant_scoped_lint.rs`: un escaneo de AST con
  `syn` sobre el `src/` del workspace, ejecutado como `cargo test`, que
  hace fallar el build si algún `#[operation]` carece de `#[idempotent]`.
  Escanea `crates/*/src` y `examples/*/src`, omite los módulos fixture
  `#[cfg(test)]`. Causa raíz: `crates/service-sdk-macros/src/lib.rs:258-259`
  fija `mutating: true` de forma codificada para todo `#[operation]`, sin
  nada a nivel de SDK que exija `mutating ⇒ idempotent`.
  Satisface: el requisito de no-bypass de extremo a extremo (toda operación
  mutante está marcada de idempotencia, estructuralmente, no solo donde una
  prueba más estrecha de la aplicación de referencia resultó mirar).
  TDD: probado en rojo (una operación mutante sin el atributo falla el
  nuevo lint) y luego en verde. Gates: `cargo fmt --check` limpio en el
  archivo nuevo, `cargo check --workspace` limpio, `cargo clippy
  --workspace --all-targets -- -D warnings` limpio, `cargo test --workspace`
  con salida 0, cero fallos.

- [x] **Fix 2 — Prueba real de carrera multi-nodo.**
  `integration-tests/tests/infrastructure/concurrent_replicas_postgres.rs`:
  ambas réplicas en competencia ahora abren un
  `EntityEventStores::open(pool.clone())` real y usan el cableado de
  producción `compose_entity_runtimes`, en lugar de que cada réplica
  escriba a través de un almacén de eventos en memoria privado. La nueva
  prueba `two_replicas_racing_one_key_yield_exactly_one_execution` afirma
  que existe exactamente un conjunto durable de eventos y un recibo
  confirmado en Postgres real para la clave en disputa — la réplica
  perdedora no escribió nada durable.
  Satisface: el escenario 8, "dos owners/nodos compitiendo" — prueba que el
  fencing de reserva que ya existía también controla las escrituras
  durables reales, no solo la propiedad de la reserva.
  Verificado en verde contra Postgres real (colima/Docker), ejecutado 3
  veces consecutivas, sin fallos intermitentes.

- [x] **Fix 3 — Prueba de recuperación tras caída de un solo agregado.**
  Nuevo archivo
  `integration-tests/tests/infrastructure/single_aggregate_crash_recovery_postgres.rs`
  (566 líneas): un proceso hijo real del sistema operativo confirma una
  operación en Postgres real (verificado vía SQL directo), luego es
  eliminado de verdad (`std::process::abort()` / SIGABRT — no simulado), y
  se demuestra que un reintento desde un proceso/pool/owner nuevo reproduce
  el resultado previo con cero re-ejecución del handler y cero filas
  duplicadas.
  Satisface: el escenario 14, "caída después del commit, antes de
  responder" — para el caso de un solo agregado, previamente probado solo
  para el caso dual-agregado.
  Verificado en verde contra Postgres real, ejecutado 3 veces consecutivas,
  sin fallos intermitentes.

- [x] **Fix 4 — Prueba real de aislamiento en Postgres.** Nuevo archivo
  `integration-tests/tests/infrastructure/receipt_identity_isolation_postgres.rs`
  (4 pruebas): mantiene fijos 3 de los 4 campos de identidad (`tenant_id`,
  `aggregate_type`, `aggregate_id`, `operation_key`) y varía uno a la vez
  contra recibos reales de Postgres, probando que no hay contaminación
  cruzada. Incluye un control negativo dentro del mismo alcance (la misma
  huella reproduce, una huella distinta genera conflicto) para que la
  prueba de aislamiento no sea vacía, y cubre explícitamente la partición
  entre el tenant NULL/systemwide y un tenant con alcance real.
  Satisface: los escenarios 17/18/19, aislamiento de tenant/tipo/id —
  previamente probado solo a nivel estructural/catálogo
  (`schema_index_assertion.rs`), nunca funcionalmente.
  Verificado en verde contra Postgres real; parte de una corrida completa
  de la suite con 39-41/41 pruebas pasando (la 1 prueba ignorada es
  preexistente/no relacionada).

- [x] **Corrección de documentación.** Se corrigieron `ROADMAP.md` §7.12 y
  `openspec/specs/idempotent-command-processing/spec.md`: el texto "dos
  adaptadores conformes — HTTP y gRPC" sobreafirmaba un segundo transporte
  de dispatch funcionando. Se reformuló para indicar que el adaptador gRPC
  (`GrpcMetadataCarrier`) es solo de portador/extracción — pasa el arnés de
  conformidad compartido para leer la clave de los metadatos, pero no
  existe en el workspace ninguna ruta de servicio/socket/dispatch de
  comandos gRPC (`crates/transport/src/lib.rs:10-32`).

## Deliberadamente Sin Marcar — Deuda Residual Documentada

- [ ] **Atomicidad de la escritura dual-agregado.** No es un defecto — es
  un no-objetivo declarado en la especificación original de PROD-012, sin
  cambios. Un reintento tras una falla parcial resume; no repite.
- [ ] **Claves de idempotencia coalescidas/duplicadas, primer-valor-gana.**
  No es un defecto — no-objetivo declarado, sin cambios. El comportamiento
  se mide y se afirma en ambos adaptadores en lugar de cerrarse.
- [ ] **Arnés genérico de conformidad de reservas no parametrizado contra
  Postgres.** Genuinamente diferido, no un no-objetivo. El arnés
  parametrizado de `testkit` que ejecutan tanto el doble en memoria como el
  adaptador de Postgres todavía no se ejecuta contra Postgres real como uno
  de sus objetivos parametrizados. Más estrecho que la brecha de
  aislamiento de tenant que el Fix 4 cerró a nivel de recibo; permanece
  abierto.
- [ ] **Valor por defecto silencioso en memoria de
  `EntityRuntimeBuilder::build()`**
  (`crates/persistent-entity/src/builder.rs:279-281`). Genuinamente
  diferido, señalado por primera vez por esta auditoría (no documentado
  antes). Ninguna ruta de producción lo alcanza hoy, pero es una trampa sin
  protección para un futuro host. Delimitado como endurecimiento futuro
  del composition root, explícitamente no un bloqueante de PROD-012 y fuera
  de alcance de este cambio.
