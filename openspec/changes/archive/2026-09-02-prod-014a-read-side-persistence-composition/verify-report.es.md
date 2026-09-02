```yaml
schema: gentle-ai.verify-result/v1
evidence_revision: sha256:1d810d170c2a6870e2ce222eb5ed95d359b06f5b36e14c8cc797a8c08a8143f5
verdict: pass
blockers: 0
critical_findings: 0
requirements: 8/8
scenarios: 18/18
test_command: cargo test --workspace
test_exit_code: 0
test_output_hash: sha256:fbb054bd2780d09ea171b790fc2e8824391726bb55325cc783a3b339c7923b82
build_command: cargo clippy --workspace -- -D warnings
build_exit_code: 0
build_output_hash: sha256:2b34696728dc82b03cbf6311d7e3a6a41658bbde097834c3010a78bc82842c1a
```

## Informe de Verificación

> Compañero de revisión en español, 1:1 con `verify-report.md` (canónico, inglés). En caso de discrepancia, `verify-report.md` es la fuente de verdad.

**Cambio**: 2026-09-01-prod-014a-read-side-persistence-composition
**Versión**: N/A (sin spec base versionado; tres deltas ADDED/MODIFIED)
**Modo**: TDD Estricto
**Contexto de esta segunda pasada**: segunda verificación, después del commit de remediación `d548aea` en `opsx/prod-014a-pr2-host`, que cierra los dos hallazgos CRITICAL de la evidencia FAIL previa `sha256:a6ab4486af4ffa979781448912d59d5f003f680006bf527151e3b9e8ab10cb62`.

### Completitud
| Métrica | Valor |
|--------|-------|
| Tareas totales | 21 |
| Tareas completas | 21 |
| Tareas incompletas | 0 |

### Ejecución de Build y Tests
**Build**: Aprobado (limpio)
```text
$ cargo clippy --workspace -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.16s
(exit 0, cero warnings, cero errores)
```

**Tests**: 1892 aprobados / 0 fallidos / 0 omitidos, en 138 binarios de test (unitarios, integración, doctest)
```text
$ cargo test --workspace
(exit 0; 138 bloques "test result: ok"; 0 apariciones de "FAILED")
```

**Cobertura**: No disponible — tarpaulin no se ejecutó en esta sesión (no forma parte del conjunto de comandos mandatados para esta fase).

Re-ejecutado de forma independiente desde la raíz del repo en HEAD `d548aea0` (árbol de trabajo limpio salvo los archivos nuevos/actualizados de este propio informe). Las cifras son +1 test respecto a la evidencia FAIL previa (1891), coincidiendo con el único test de remediación agregado en el commit `d548aea`.

### Matriz de Cumplimiento de Especificación

**application-composition** (3 requisitos / 7 escenarios)

| Requisito | Escenario | Test | Resultado |
|-----------|-----------|------|-----------|
| Registro del par de progreso durable read-side, indexado por projection_id | Dos proyecciones registran pares distintos de forma independiente | `app::mod::tests::read_side_progress_registration_for_two_different_projections_both_succeed` | CUMPLE |
| (mismo) | El registro parcial de un solo store no es representable | Forma de la API: `AppBuilder::read_side_progress(projection_id, offset, dedup)` es el único punto de entrada público, un solo método que toma ambos stores juntos (inspección de código, `app/mod.rs`) | CUMPLE (propiedad estructural/de compilación) |
| (mismo) | La misma instancia de store puede compartirse entre projection_ids | `app::mod::tests::read_side_progress_registration_with_a_shared_store_instance_across_two_projection_ids_both_succeed` — registra UN solo resultado de `stub_pair()`, clona el mismo `Arc<dyn OffsetStore>`/`Arc<dyn DedupStore>` en ambas llamadas `.read_side_progress()` bajo dos `projection_id` distintos, verifica `Ok` | **CUMPLE** (era UNTESTED en la evidencia previa `sha256:a6ab4486af...`; cerrado por el commit de remediación `d548aea`) |
| El registro duplicado del progreso durable read-side a través de AppBuilder falla cerrado | El registro duplicado para el mismo projection_id aflora en build | `app::mod::tests::duplicate_read_side_progress_registration_is_rejected` | CUMPLE |
| (mismo) | Un error de composición preexistente no es sobrescrito por un registro posterior | `app::mod::tests::read_side_progress_short_circuits_on_a_pending_error` | CUMPLE |
| Un par de progreso durable registrado es el par que la proyección realmente usa | El par registrado es el par con el que la proyección se genera (spawns) | `build_runtime_with` (lib.rs) clona el mismo valor `progress` en `AppBuilder::read_side_progress(...)` y en `ReadSideHandles::new(store, progress.clone())` (inspección de código); ningún test verifica directamente identidad de puntero, coincidiendo con el alcance que design.md define para esta propiedad como estructural, no verificada por test | CUMPLE (estructural, según el alcance declarado en design.md) |
| (mismo) | El camino de Producción del host de referencia obtiene su par desde la raíz de composición | `production_profile_guard::production_profile_with_durable_read_side_progress_registers_and_builds` | CUMPLE |

**production-composition-hardening** (4 requisitos / 8 escenarios)

| Requisito | Escenario | Test | Resultado |
|-----------|-----------|------|-----------|
| Compuerta de progreso durable read-side bajo Producción | Un store volátil en un par registrado es rechazado en el bootstrap | `runtime::builder::tests::validate_read_side_progress_profile_rejects_volatile_offset` / `_rejects_volatile_dedup` / `_rejects_both_volatile`, más `read_side_progress_composition::app_builder_surfaces_a_volatile_read_side_progress_pair_as_composition_validation_error`, más `production_profile_guard::production_profile_with_volatile_read_side_progress_is_refused` | CUMPLE |
| (mismo) | Sin proyección registrada no hay nada que compuertar | `runtime::builder::tests::validate_read_side_progress_profile_ok_when_none_registered` | CUMPLE |
| (mismo) | Ambos stores durables tiene éxito | `runtime::builder::tests::validate_read_side_progress_profile_ok_when_pair_durable`, `production_profile_guard::production_profile_with_durable_read_side_progress_registers_and_builds` | CUMPLE |
| (mismo) | El perfil Dev con stores volátiles no cambia | `runtime::builder::tests::validate_read_side_progress_profile_ok_under_dev_with_volatile_pair`, `read_side_progress_composition::app_builder_accepts_a_volatile_read_side_progress_pair_under_dev_profile` | CUMPLE |
| El comentario de doc de `Profile::Production` refleja la cuarta capacidad gobernada | El comentario de doc lista la cuarta capacidad gobernada | `crates/persistent-entity/src/profile.rs` líneas 18-27 (inspección de código — texto de comentario, no cubierto por test por naturaleza) | CUMPLE (estructural) |
| Un único predicado compartido gobierna todas las capacidades de persistencia | Las tres capacidades preexistentes enrutan su decisión por el mismo predicado (sin cambios) | `require_durably_configured_matrix`, `presence_alone_is_not_durability` (ambos preexistentes, sin modificar) | CUMPLE |
| (mismo) | La decisión de la cuarta capacidad enruta por el mismo predicado | `validate_read_side_progress_profile` llama a `persistent_entity::profile::require_durably_configured` (inspección de código, `builder.rs` línea ~881) | CUMPLE |
| Los rechazos son accionables | El error nombra la capacidad y la corrección | `validate_read_side_progress_profile_rejects_volatile_offset` verifica que el mensaje contiene `"read-side progress"` y `"read_side_progress"` | CUMPLE |

**read-side** (1 requisito / 3 escenarios)

| Requisito | Escenario | Test | Resultado |
|-----------|-----------|------|-----------|
| Aceptación en la raíz de composición sin cambio en la capa del scheduler | La raíz de composición clasifica y valida sin construir | Propiedad de diff — sin cambio de código en `TagSchedulerImpl`/`ProjectionSpec`; la construcción se mueve al host (`ReadSideProgressStores::in_memory()`/`fake_durable()`), nunca al framework (inspección de código) | CUMPLE (estructural, según el alcance propio de design.md) |
| (mismo) | Una aplicación que no registra nada no se ve afectada | `production_profile_guard::dev_profile_still_builds_at_the_composition_root`, `runtime::builder::tests::validate_read_side_progress_profile_ok_when_none_registered` | CUMPLE |
| (mismo) | El rechazo nunca llega al motor del scheduler | `read_side_progress_composition::app_builder_surfaces_a_volatile_read_side_progress_pair_as_composition_validation_error` — los stores de prueba volátiles en este test entran en pánico con `unreachable!()` en cada método del store; el test pasa sin pánico, probando que el rechazo ocurre antes de cualquier acceso al store | CUMPLE |

**Resumen de cumplimiento**: 18/18 escenarios cumplen, 0 UNTESTED

### Corrección (Evidencia Estática)
| Requisito | Estado | Notas |
|-----------|--------|-------|
| `is_durable()` por defecto y forwarding de `Arc<T>` en `OffsetStore`/`DedupStore` (AD-3/AD-4) | Implementado | `offset.rs`/`dedup.rs`, coincide byte a byte con el diseño |
| División de registro + validador en `RuntimeBuilder` (AD-6) | Implementado | `validate_persistence_profile` llama a `validate_effect_store_profile()` y luego a `validate_read_side_progress_profile()`; la regresión EC-1 está explícitamente cubierta |
| Registro + guardia de duplicados en `AppBuilder` (AD-7) | Implementado | Patrón de cerrojo `pending_error` reutilizado textualmente; el mensaje de `CompositionError::DuplicateReadSideProgress` coincide con el diseño |
| Recableado del host reference-app (AD-8/AD-9) | Implementado | `ReadSideProgressStores`, `FakeDurableOffsetStore`/`FakeDurableDedupStore`, el parámetro `Option<ReadSideProgressStores>` de `build_runtime_with`, el `None` explícito de `main.rs` con comentario de justificación F-1 |
| Comentario de doc de `Profile::Production` (AD-10) | Implementado | Coincidencia textual, incluyendo ambos límites declarados |
| 13 actualizaciones mecánicas de sitios de llamada | Implementado | Los 13 confirmados actualizados (5x `ReadSideHandles::new`, 8x `build_runtime_with`), incluyendo el workspace de excepción a nivel raíz `integration-tests/` |
| Cobertura de test de instancia compartida entre projection_ids (remediación) | Implementado | `read_side_progress_registration_with_a_shared_store_instance_across_two_projection_ids_both_succeed` llama a `stub_pair()` una sola vez, clona sus `Arc` en ambas llamadas de registro, verifica `Ok`; re-inspeccionado de forma independiente en esta pasada, no solo re-ejecutado |

### Coherencia (Diseño)
| Decisión | ¿Seguida? | Notas |
|----------|-----------|-------|
| AD-1: el par como unidad de registro, indexado por `projection_id` | Sí | `BTreeMap<String, ReadSideProgressPair>`, un método combinado |
| AD-3: impls de forwarding `Arc<dyn Trait + ?Sized>` | Sí | En `offset.rs` y `dedup.rs`, cubiertos con pares `bare_impl_defaults_is_durable_to_false` + `arc_forwards_is_durable` |
| AD-5: `BTreeMap` en vez de `HashMap` para reporte determinista del primer infractor | Sí | Tipo de campo confirmado en `builder.rs` |
| AD-6: división del validador, chequeo de effect-store primero | Sí | Secuencia confirmada en `validate_persistence_profile` |
| AD-7: patrón de cerrojo de guardia de duplicados | Sí | Replica exactamente el precedente `adapter_types`/`DuplicateEffectStore` |
| AD-8: el mismo valor alimenta registro y spawn | Sí | Una sola variable `progress` clonada en ambos destinos dentro de `build_runtime_with` |
| AD-9: `FakeDurable*` como newtypes delegados delgados | Sí | Confirmado en `read_side/store.rs` |
| AD-10: el comentario de doc nombra la cuarta capacidad y sus dos límites | Sí | Coincidencia textual |
| Sin cambio de código en la capa del scheduler (`TagSchedulerImpl`/`ProjectionSpec`) | Sí | Confirmado vía diff — cero líneas modificadas en archivos del scheduler |
| Campo "Chain strategy" de tasks.md resuelto | **Sí (corregido en la remediación)** | Ahora dice `stacked-to-main`, coincidiendo con la topología real de ramas (`opsx/prod-014a-pr2-host` apilada sobre `opsx/prod-014a-pr1-framework` sobre `develop`); antes quedaba como `pending` |

### TDD Estricto — Secciones Adicionales

#### Cumplimiento TDD
| Chequeo | Resultado | Detalles |
|---------|-----------|----------|
| Evidencia TDD reportada | **SÍ** | El artefacto de apply-progress (observación Engram #1640, revisión 3) ahora contiene una tabla completa "TDD Cycle Evidence" con columnas Task / Test File / Layer / Safety Net / RED / GREEN / TRIANGULATE / REFACTOR para las 21 tareas originales más la nueva fila de remediación `3.3-R`, coincidiendo exactamente con el esquema de `strict-tdd-verify.md` |
| Todas las tareas tienen tests | Sí | 21/21 tareas mapean a un archivo o función de test identificable |
| RED confirmado (los tests existen) | Sí | Cada archivo/función de test nombrado en la tabla existe en el código, incluida la nueva fila `3.3-R` (confirmado por lectura directa de `app/mod.rs` líneas 1913-1928) |
| GREEN confirmado (los tests pasan) | Sí | 1892/1892 pasaron en la re-ejecución independiente, 0 fallidos |
| Triangulación adecuada | Sí | La matriz Producción/Dev x {ninguno, durable, offset-volátil, dedup-volátil, ambos-volátiles} en `builder.rs` triangula 6 casos distintos; la fila de remediación `3.3-R` se reporta correctamente como escenario de caso único |
| Red de seguridad para archivos modificados | Sí | La fila `3.3-R` reporta explícitamente "13/13 tests preexistentes de `app::` read-side-progress en verde antes de agregar" |

**Cumplimiento TDD**: 6/6 chequeos completamente confirmados (ambos huecos procedimentales de la evidencia FAIL previa están cerrados)

#### Distribución por Capa de Test
| Capa | Tests | Archivos | Herramientas |
|------|-------|----------|---------------|
| Unitarios | ~19 | `offset.rs`, `dedup.rs`, `app/mod.rs`, `app/error.rs`, `runtime/builder.rs`, `profile.rs` | `#[cfg(test)]`, `cargo test` plano |
| Integración | ~5 | `crates/service-sdk/tests/read_side_progress_composition.rs`, `examples/reference-app/tests/production_profile_guard.rs` | `cargo test`, sin servicios externos |
| E2E | 1 | `examples/reference-app/tests/users_by_tenant_projection.rs::projection_populates_from_real_registration_events_not_a_hand_built_read_model` | `TagSchedulerImpl` real, en proceso |
| **Total** | **~25** (subconjunto acotado al cambio, de 1892 en todo el workspace) | 8 archivos | — |

#### Cobertura de Archivos Modificados
Análisis de cobertura omitido — no se ejecutó tarpaulin en esta sesión; no forma parte del conjunto de comandos mandatados para esta fase.

#### Calidad de Aserciones
✅ Todas las aserciones revisadas verifican comportamiento real, incluyendo el nuevo test de remediación: `read_side_progress_registration_with_a_shared_store_instance_across_two_projection_ids_both_succeed` llama a código de producción real (`compat_app().read_side_progress(...).read_side_progress(...).build()`) y verifica `Ok(_)` con un `panic!` en `Err`, no una tautología ni un smoke-test. Se re-escanearon los 8+1 archivos de test acotados al cambio en busca de tautologías — cero encontradas.

**Calidad de aserciones**: 0 CRITICAL, 0 WARNING

#### Métricas de Calidad
**Linter (clippy)**: Sin errores, sin warnings (`-D warnings` limpio)
**Complejidad cognitiva**: Sin warning de `clippy::cognitive-complexity` en `ego-service-sdk` (donde `validate_persistence_profile` se dividió según la tarea 2.3); el único warning de complejidad preexistente en el workspace está en `persistent-entity/src/actor.rs::execute_command` (archivo no relacionado, sin tocar por este cambio)

### Completitud de Artefactos Bilingües
Todos los artefactos canónicos en inglés tienen su compañero `.es.md` proporcional: `proposal.md`/`.es.md`, `tasks.md`/`tasks.es.md` (ambos actualizados de forma idéntica para la corrección de chain-strategy), y los 3 deltas de spec (`spec.md`/`spec.es.md`). `design.md` sigue sin compañero `.es.md` — sin cambios respecto a la evidencia previa, ver WARNING abajo.

### Problemas Encontrados

**CRITICAL**: Ninguno

Ambos hallazgos CRITICAL de la evidencia previa `sha256:a6ab4486af4ffa979781448912d59d5f003f680006bf527151e3b9e8ab10cb62` se confirman cerrados de forma independiente:
1. El escenario de spec "la misma instancia de store puede compartirse entre projection_ids" ahora tiene un test que pasa en runtime (`read_side_progress_registration_with_a_shared_store_instance_across_two_projection_ids_both_succeed`), leído e independientemente confirmado que registra un `Arc` idéntico bajo dos `projection_id` distintos y verifica éxito — no dos instancias de stub distintas como antes.
2. El artefacto apply-progress ahora tiene la tabla mandatoria "TDD Cycle Evidence" en el esquema exacto que exige `strict-tdd-verify.md`, leída e independientemente confirmada que cubre las 21 tareas originales más la nueva fila de remediación.

**WARNING**:
1. PR1 (`opsx/prod-014a-pr1-framework` vs `origin/develop`) mide 908 líneas modificadas, superando el presupuesto de 400 líneas por carga de revisión; no existe un registro de gobernanza `size:exception` discreto más allá de una mención de paso en la sección "Learned" de apply-progress. PR2 (`opsx/prod-014a-pr2-host` vs su base `opsx/prod-014a-pr1-framework`) mide 358 líneas modificadas (312 inserciones + 46 eliminaciones), incluyendo el commit de remediación de +27 líneas, todavía cómodamente por debajo del presupuesto.
2. Ninguna de las dos PR ha sido subida (push) ni abierta todavía (ambas permanecen solo locales en `opsx/prod-014a-pr2-host` apilada sobre `opsx/prod-014a-pr1-framework` sobre `develop`). Esperado en esta fase, no un vacío funcional — se anota para que la fase de archivo no asuma que ya están activas.
3. `design.md` no tiene compañero `.es.md` en la carpeta del cambio, a diferencia de todos los demás artefactos canónicos (proposal, specs, tasks) — una inconsistencia menor y preexistente en la convención bilingüe, no afectada por esta remediación.
4. **Hueco de vinculación en el ledger (proceso, no código)**: el intento de remediación del ledger nativo `sdd-attempt` (ordinal 5, work-unit `remediate-verify-criticals`, evidence-revision `sha256:a921cfad0d431b3d92d49bc9ce3f5dc5015861eb949d4a5c717aab43289dcbde`) se asentó (settle) exitosamente pero sin un enlace `--remediates-evidence-revision` de vuelta a la evidencia FAILED `sha256:a6ab4486af...`, porque 3 intentos de pasar esa bandera en esa llamada de settle devolvieron `blocked: invalid_continuation` (un punto de fricción real en el chequeo de elegibilidad de continuación del CLI, no un problema de datos — el settle tuvo éxito una vez que se quitó la bandera). Un intento previo (ordinal 4, mismo work-unit) también se asentó con texto placeholder `"test"` en `diagnosis`/`cleanup_evidence`/`process_evidence` antes de ser reiniciado por una entrada `last_reset` con alcance de mantenedor — visible en `gentle-ai sdd-attempt status` y una recuperación legítima, no pérdida de datos. Esta pasada de verificación adjunta `--remediates-evidence-revision sha256:a6ab4486af...` a su propio settle para cerrar ese enlace en la capa correcta (un intento de verificación exitoso que sustituye a uno fallido), ya que no es posible editar retroactivamente un intento ya asentado. Si `gentle-ai sdd-continue` sigue reportando `next_recommended: remediate` apuntando a la evidencia antigua después de leer el settle de esta pasada, es un defecto del lado del CLI a reportar aguas arriba, no un defecto en el código o la evidencia de test subyacente, que la inspección independiente en esta pasada confirma real y correcta.

**SUGGESTION**: Ninguna (las dos sugerencias de la evidencia previa fueron ambas resueltas por el commit de remediación).

### Veredicto
PASS
21/21 tareas completas, 18/18 escenarios de spec confirmados de forma independiente como CUMPLE con tests que pasan en runtime (antes 17/18), 1892/1892 tests pasando (0 fallidos), 0 warnings de clippy, y la tabla de evidencia TDD Estricto mandatoria está presente y verificada independientemente contra el código. Ambos hallazgos CRITICAL previos están cerrados por el commit `d548aea`, confirmado por inspección directa de código en esta pasada, no solo confiando en las afirmaciones de la propia remediación. Los ítems restantes son notas de proceso/gobernanza a nivel WARNING (presupuesto/estado de push de las PR, un `design.es.md` faltante, y un hueco de vinculación en el ledger nativo que es fricción de CLI, no un defecto de código o evidencia).
