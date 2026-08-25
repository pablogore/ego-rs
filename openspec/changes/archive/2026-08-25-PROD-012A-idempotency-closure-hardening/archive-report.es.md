# Reporte de Archivo: PROD-012A — Endurecimiento de Cierre de Idempotencia

**Cambio**: `2026-08-25-PROD-012A-idempotency-closure-hardening`
**Auditado/Archivado**: 2026-08-25
**Rama / SHA auditado**: `develop` @ `a740d3476eb704d216a37d45961e8bde1c19aeca`
**Estado**: Completo

## Resumen Ejecutivo

Una auditoría de seguimiento sobre la garantía ya archivada de PROD-012
("Procesamiento Idempotente de Comandos de Extremo a Extremo", archivada el
2026-08-20) encontró que el mecanismo central era sólido — la comparación de
huella (fingerprint), los recibos `NoEvents`, el CAS de fencing, y la
recuperación tras caída dual-agregado están todos probados contra
PostgreSQL real — pero encontró un bypass estructural y tres lugares donde
la prueba era más delgada que la afirmación. Los cuatro ya fueron cerrados
con evidencia real. Este cambio se registra como su propio seguimiento
atómico en lugar de una edición retroactiva del archivo congelado del
2026-08-20, según la convención de este proyecto de que los cambios
archivados son historia congelada.

## Qué Se Encontró

1. **Bypass estructural**: `#[operation]` fija `mutating: true` de forma
   codificada (`crates/service-sdk-macros/src/lib.rs:258-259`);
   `#[idempotent]` era completamente opcional, sin exigencia a nivel de SDK
   de `mutating ⇒ idempotent`.
2. **Brecha de carrera multi-nodo**: la prueba de réplicas concurrentes
   probaba el fencing de reservas contra Postgres real, pero las
   escrituras reales de eventos de cada réplica pasaban por un almacén en
   memoria privado — la garantía real de "solo una escritura durable
   sobrevive" no estaba probada de extremo a extremo.
3. **Brecha de recuperación tras caída de un solo agregado**: la
   recuperación tras caída-después-del-commit estaba probada solo para el
   caso dual-agregado.
4. **Brecha de alcance de aislamiento**: el aislamiento de
   tenant/aggregate_type/aggregate_id estaba probado solo a nivel
   estructural/catálogo, nunca funcionalmente contra recibos reales de
   Postgres.
5. **Desvío de documentación** (no una brecha de código): el ROADMAP y la
   especificación afirmaban "dos adaptadores conformes — HTTP y gRPC" para
   el dispatch de comandos; solo HTTP despacha comandos reales, el
   adaptador gRPC es solo de portador/extracción.

## Qué Se Corrigió

- **Fix 1**: Nuevo `crates/service-sdk/tests/idempotent_marker_lint.rs` —
  un escaneo de AST con `syn` (que refleja `tenant_scoped_lint.rs`) que
  hace fallar el build estructuralmente si algún `#[operation]` carece de
  `#[idempotent]`, sobre `crates/*/src` y `examples/*/src`.
- **Fix 2**: `integration-tests/tests/infrastructure/concurrent_replicas_postgres.rs`
  ahora hace que ambas réplicas en competencia escriban a través de un
  `EntityEventStores::open(pool.clone())` real y compartido, y del cableado
  de producción `compose_entity_runtimes`. La nueva prueba
  `two_replicas_racing_one_key_yield_exactly_one_execution` confirma que
  existe exactamente un conjunto durable de eventos y un recibo confirmado
  para la clave en disputa.
- **Fix 3**: Nuevo
  `integration-tests/tests/infrastructure/single_aggregate_crash_recovery_postgres.rs`
  (566 líneas) — un proceso hijo real confirma en Postgres real, es
  eliminado de verdad (`std::process::abort()`), y se demuestra que un
  proceso/pool/owner nuevo reproduce el resultado con cero re-ejecución y
  cero filas duplicadas.
- **Fix 4**: Nuevo
  `integration-tests/tests/infrastructure/receipt_identity_isolation_postgres.rs`
  (4 pruebas) — mantiene fijos 3 de 4 campos de identidad y varía uno a la
  vez contra recibos reales de Postgres, con un control negativo y
  cobertura explícita del tenant NULL/systemwide.
- **Documentación**: se corrigieron `ROADMAP.md` §7.12 y
  `openspec/specs/idempotent-command-processing/spec.md` para indicar que
  el adaptador gRPC es solo de portador/extracción, sin una ruta real de
  dispatch de comandos gRPC en el workspace
  (`crates/transport/src/lib.rs:10-32`).

## Resultados de los Gates

| Gate | Resultado |
|------|-----------|
| `cargo fmt --check` (archivo nuevo) | Limpio |
| `cargo check --workspace` | Limpio |
| `cargo clippy --workspace --all-targets -- -D warnings` | Limpio |
| `cargo test --workspace` | Salida 0, cero fallos |
| Prueba del Fix 2, 3 corridas consecutivas contra Postgres real | Verde, sin fallos intermitentes |
| Prueba del Fix 3, 3 corridas consecutivas contra Postgres real | Verde, sin fallos intermitentes |
| Pruebas del Fix 4 contra Postgres real | Verde |
| Suite de integración completa | 39-41/41 pasando (1 prueba ignorada preexistente/no relacionada) |

## Brechas Residuales (Documentadas, No Bloqueantes)

- Atomicidad de la escritura dual-agregado — no-objetivo declarado, sin
  cambios.
- Claves de idempotencia coalescidas/duplicadas admitidas
  primer-valor-gana — no-objetivo declarado, sin cambios.
- El arnés genérico de conformidad de reservas (`testkit`) todavía no se
  ejecuta contra Postgres real como uno de sus objetivos parametrizados —
  más estrecho que la brecha de aislamiento de tenant que cerró el Fix 4, y
  genuinamente diferido en lugar de un no-objetivo.
- `EntityRuntimeBuilder::build()`
  (`crates/persistent-entity/src/builder.rs:279-281`) todavía cae
  silenciosamente en un almacén de eventos en memoria no durable cuando
  nunca se llama a `.with_event_store()`. Ninguna ruta de producción lo
  alcanza hoy; señalado por primera vez por esta auditoría, delimitado como
  endurecimiento futuro del composition root, no un bloqueante de
  PROD-012.

## Recomendación

Los cuatro escenarios/invariantes que apuntó este endurecimiento —
exigencia de no-bypass de extremo a extremo, carrera a nivel de escritura
entre dos nodos, recuperación tras caída-después-del-commit de un solo
agregado, y aislamiento de recibos por tenant/tipo/id — ya están
demostrados con evidencia real contra PostgreSQL real, cerrando la brecha
entre lo que PROD-012 afirmaba y lo que estaba probado. Los ítems
residuales de arriba son no-objetivos documentados o deuda estrecha y no
bloqueante, no violaciones de la garantía central. Se recomienda archivar
este cambio en su estado actual, ya completo — no se requiere trabajo
adicional para cerrarlo.

## Autoridad y Cierre

- **Auditado por**: endurecimiento del 2026-08-25, registrado como este
  seguimiento atómico al archivo congelado de PROD-012 del 2026-08-20.
- **Autoridad de tareas**: `tasks.md` en esta carpeta de cambio.
- **Fecha de archivo**: 2026-08-25
