# Tareas: PROD-014A — Composición del Progreso Duradero del Read-Side

> Compañero de revisión en español. Fuente canónica: `tasks.md` (identificadores 1:1).
> TDD estricto: toda tarea es RED (test que falla) antes de GREEN. `cargo clippy --workspace -- -W clippy::cognitive-complexity` tras cada división.

## Pronóstico de Carga de Revisión

| Campo | Valor |
|-------|-------|
| Líneas cambiadas estimadas | ~450–600 (2 traits modificados, cambios en `RuntimeBuilder`+`AppBuilder`, 13 sitios de llamada mecánicos, nuevos tipos en el host, tests unit/integration/E2E) |
| Riesgo de presupuesto de 400 líneas | Alto |
| PRs encadenados recomendados | Sí |
| División sugerida | PR 1 (framework) → PR 2 (host) |
| Estrategia de entrega | ask-on-risk |
| Estrategia de cadena | stacked-to-main — confirmada por la topología real de ramas (`opsx/prod-014a-pr2-host` apilada sobre `opsx/prod-014a-pr1-framework` sobre `develop`) |

Decisión necesaria antes de aplicar: Sí
PRs encadenados recomendados: Sí
Estrategia de cadena: stacked-to-main
Riesgo de presupuesto de 400 líneas: Alto

### Unidades de Trabajo Sugeridas

| Unidad | Objetivo | PR probable | Comando de test enfocado | Arnés de ejecución | Límite de rollback |
|--------|----------|--------------|---------------------------|---------------------|----------------------|
| 1 | Framework: `is_durable()` + forwarding de `Arc<T>` en ambas SPIs; registro + división del validador en `RuntimeBuilder`; registro + guardia de duplicados en `AppBuilder` | PR 1 | `cargo test -p ego-domain read_side`, `cargo test -p ego-service-sdk read_side_progress` | N/A — solo unit/integration, no requiere host para probar el PR 1 | Revertir los dos defaults del trait, los impls de `Arc<T>`, los campos+métodos de `RuntimeBuilder`/`AppBuilder`, la variante de `CompositionError`; nada más depende de ellos todavía |
| 2 | Host: reconexión de `ReadSideProgressStores`/`FakeDurable*` en reference-app, 13 sitios de llamada mecánicos, `main.rs` con `None`, corrección de doc en `profile.rs` | PR 2 | `cargo test -p reference-app production_profile_guard` | Escenario existente de `examples/reference-app` en perfil Dev (sin arnés nuevo) | Revertir las firmas de `ReadSideHandles::new`/`build_runtime_with` y los 13 sitios de llamada; el PR 1 sigue siendo válido con cero registros |

## Fase 1: SPIs del Dominio (Fundación) — PR 1

- [x] 1.1 RED `crates/domain/src/read_side/offset.rs`: un impl desnudo tiene `is_durable()` en `false` por defecto; `Arc::new(durable).is_durable() == true`; verificar primero `?Sized` bajo `#[async_trait]` (trampa de AD-3), usar `T: ... + 'static` si resulta impráctico
- [x] 1.2 GREEN: agregar `is_durable(&self) -> bool { false }` por defecto + `impl OffsetStore for Arc<T>` reenviando `read_offset`/`write_offset`/`is_durable` (AD-3, AD-4)
- [x] 1.3 RED+GREEN: replicar 1.1–1.2 en `dedup.rs` para `DedupStore`

## Fase 2: Registro en RuntimeBuilder + División del Validador — PR 1

- [x] 2.1 RED `crates/service-sdk/src/runtime/builder.rs`: matriz {Dev,Production}×{ninguno, durable, offset-volátil, dedup-volátil, ambos-volátiles}; asegurar la regresión de EC-1 — cero effect executors + read-side volátil sigue rechazado; `build()`/`try_build()` coinciden
- [x] 2.2 GREEN: agregar campo `read_side_progress: BTreeMap<String, ReadSideProgressPair>`, `ReadSideProgressPair{offset,dedup}` privado, `with_read_side_progress(...)`
- [x] 2.3 GREEN: dividir `validate_persistence_profile` en `validate_effect_store_profile` (sin cambios) + nueva `validate_read_side_progress_profile` (AD-6); el secuenciador llama a ambas, effect store primero
- [x] 2.4 Agregar stubs mínimos `#[cfg(test)]` durable/volátil de `OffsetStore`/`DedupStore` para la matriz (AD-9, lado framework)

## Fase 3: Registro en AppBuilder + Guardia de Duplicados — PR 1

- [x] 3.1 RED `crates/service-sdk/src/app/error.rs`: el mensaje de `DuplicateReadSideProgress` nombra el `projection_id`, sin sugerir una API de reemplazo
- [x] 3.2 GREEN: agregar `CompositionError::DuplicateReadSideProgress { projection_id }`
- [x] 3.3 RED `crates/service-sdk/src/app/mod.rs`: el mismo `projection_id` dos veces falla cerrado en `build()` con el primer registro intacto; dos ids distintos se registran ambos; un `pending_error` preexistente no se sobrescribe
- [x] 3.4 GREEN: agregar `read_side_progress_ids: HashSet<String>` + `read_side_progress(projection_id, offset, dedup)` (latch antes de delegar a `RuntimeBuilder`)
- [x] 3.5 Test de integración en `crates/service-sdk/tests/`: el rechazo aparece como `CompositionError::Validation(RuntimeError::PersistenceNotConfigured(..))` a través de todo el camino de `build()`

## Fase 4: Reconexión de Reference-App — PR 2

- [x] 4.1 RED+GREEN `examples/reference-app/src/read_side/store.rs`: `FakeDurableOffsetStore`/`FakeDurableDedupStore` delegan a `InMemory*`, sobrescriben `is_durable() -> true` (AD-9)
- [x] 4.2 `read_side/mod.rs`: agregar `ReadSideProgressStores{offset,dedup}` con `in_memory()`/`fake_durable()`; cambiar `ReadSideHandles::new(store, progress)` (AD-8)
- [x] 4.3 `lib.rs`: `build_runtime_with` gana `read_side_progress: Option<ReadSideProgressStores>`; `None` → `in_memory()` sin registro; `Some(pair)` registra vía `AppBuilder::read_side_progress(PROJECTION_ID, ..)` y pasa el mismo clon a `ReadSideHandles::new`
- [x] 4.4 `main.rs`: pasar `None` (no existe backend durable — F-1)
- [x] 4.5 Mecánico: actualizar los 13 sitios de llamada según la lista de Radio de Impacto del diseño (5× `ReadSideHandles::new`, 8× `build_runtime_with`, en `tests/` e `integration-tests/`)
- [x] 4.6 `crates/persistent-entity/src/profile.rs`: reemplazar el comentario de doc de `Profile::Production` literalmente (AD-10, IS-10) — sin cambio de firma

## Fase 5: Verificación End-to-End — PR 2

- [x] 5.1 Actualizar `examples/reference-app/tests/users_by_tenant_projection.rs`: el par erasado fluye por `ReadSideProgressStores` hasta el scheduler real (AD-3 funciona de punta a punta)
- [x] 5.2 Actualizar `examples/reference-app/tests/production_profile_guard.rs`: `None` sigue compilando en Dev; `Some(fake_durable())` registra y compila en Production; un par volátil registrado es rechazado
- [x] 5.3 `cargo test --workspace` sin fallos; `cargo clippy --workspace -- -D warnings` limpio; confirmar que ninguna función de 2.3 supera complejidad 10
