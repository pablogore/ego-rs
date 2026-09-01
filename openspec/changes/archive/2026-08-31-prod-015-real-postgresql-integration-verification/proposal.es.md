# Propuesta: PROD-015 — Verificación de Integración contra PostgreSQL Real

> Documento acompañante para revisión. La fuente de verdad canónica es `proposal.md` (identificadores 1:1).

## Objetivo

Cerrar los invariantes que solo un PostgreSQL real — migraciones reales, transacciones reales,
bloqueos de fila reales, concurrencia real — puede demostrar, y que hoy el workspace no
asegura en ningún lugar. Todo invariante en alcance es una garantía de SQL, transacción o
concurrencia de PostgreSQL. Nada más es admitido.

## Intención

`integration-tests/` ya existe como un workspace de Cargo independiente con 16 tests admitidos
(PROD-012 / PROD-012A / PROD-002-G11). PROD-015 **extiende esa estructura existente**: no la
crea, no levanta un workspace nuevo y no toca `cargo test --workspace`, que sigue sin Docker.

Lo que queda abierto es acotado y verificado contra HEAD, no heredado de un backlog obsoleto:

- **Los harnesses de conformidad siguen siendo solo en memoria.**
  `assert_event_store_conformance` (`crates/testkit/src/event_store.rs:69`) solo es ejecutado
  por `crates/infrastructure/tests/in_memory_event_store_conformance.rs` y
  `crates/persistent-entity/tests/default_store_conformance.rs`;
  `assert_reservation_store_conformance` (`crates/testkit/src/reservation_conformance.rs:963`)
  solo por `crates/testkit/src/reservation.rs`. Los adaptadores durables nunca fueron sometidos
  a las mismas definiciones. `integration-tests/README.md` ya declara que reutilizarlos es una
  convención de esta suite; es una convención sin ningún call site.
- **La concurrencia optimista de la propia tabla `events` no está guardada.**
  `conflict_from_postgres.rs` carga la unicidad de `(tenant_id, operation_key)` en la tabla de
  *reservas*. Ningún test compite por appends sobre un stream.
- **La carrera de N contendientes por un lease no está guardada.**
  `fencing_window_postgres.rs` prueba la re-verificación bajo bloqueo de fila con un solo
  contendiente. El propio `integration-tests/README.md` registra la carrera de muchos
  contendientes como todavía faltante.
- **La migración 007 y su backfill tienen cero cobertura contra PostgreSQL real.**
  `crates/persistence/src/postgres/aggregate_type_backfill.rs` está vivo; solo su lógica pura
  `split_aggregate_id` tiene tests unitarios. Su comportamiento transaccional no está probado.
- **La semántica de `NULL` para tenant se asegura estructuralmente, no de forma conductual.**
  `schema_index_assertion.rs` lee el catálogo; nada ejercita `Option::None` contra la lógica
  de tres valores en la identidad de stream de `events`.

`docs/integration-test-backlog.md` y el issue #275 son ambos anteriores al trabajo ya entregado
y describen tests que hoy existen. No son fuente de verdad para este cambio; el árbol lo es.

## Decisiones Activas

| ID | Decisión | Justificación |
|----|----------|---------------|
| D-1 | El identificador es **PROD-015**, no PROD-014 | `PROD-014 — Read-Side Persistence Composition & Durable Store` ya está reservado en ROADMAP.md §7.13 y comprometido por PROD-013 D-5 |
| D-2 | El alcance es **solo PostgreSQL**. Verificación de transporte, HTTP, socket y OTLP queda totalmente excluida (OOS-1) | Los 13 criterios formales de aceptación de #275 están acotados a PostgreSQL, al script guardia y a los harnesses de testkit. El transporte aparece únicamente en la sección descriptiva "Scope by category §7" y en "Subsequent partitions", que el propio issue admite "solo si genuinamente requiere infraestructura real". Además esos ítems son *loopback hermético* según la clasificación propia de este repo (`skills/testing/SKILL.md` Regla 3, `skills/testing-strategy/SKILL.md` "Self-hosted loopback is not external infrastructure"): no necesitan contenedor alguno, así que agruparlos aquí sería incorrecto tanto por ubicación como por atomicidad |
| D-3 | **El límite de agotamiento del fencing en `i64::MAX` queda resuelto FUERA de alcance — ya cubierto, sin test nuevo.** | Verificado en lugar de asumido: `token_for_storage` / `token_from_storage` (`crates/persistence/src/postgres/reservation.rs:107,124`) ya tienen tests unitarios en proceso en `:626-665` que aseguran que `i64::MAX` es almacenable y que `i64::MAX + 1` es rechazado. La forma de la columna `BIGINT` + `CHECK (fencing_token > 0)` ya está cubierta por la categoría existente de esquema/catálogo. PostgreSQL real no agrega nada |
| D-4 | **Se espera que la atomicidad de la unidad de trabajo quede satisfecha por IS-1, no por un test propio.** | `assert_event_store_conformance` ya asegura que soltar sin commit descarta, y que un append en stage es invisible para un `store.load()` emitido fuera de la unidad de trabajo (`crates/testkit/src/event_store.rs:328-375`). Contra `PostgreSQLEventStore` esa lectura viaja por una *conexión distinta del pool*, con lo que se vuelve gratis una aserción real de aislamiento entre conexiones. El diseño confirma la propiedad de conexión distinta; solo si no se sostiene se justifica un test aparte |
| D-5 | El harness de reservas **no necesita cambios en testkit** para correr contra PostgreSQL | Verificado: `assert_reservation_store_conformance` recibe una factory `Fn() -> (S, Arc<TestClock>)`, y `PostgresOperationReservationStore::new(pool, clock)` (`reservation.rs:85`) ya acepta un `Arc<dyn Clock>` inyectado. Las formas ya encajan |
| D-6 | **Sin objetivo de cantidad de tests.** Aproximadamente cinco o seis archivos nuevos, cada uno ganándose su lugar | El propio encuadre de #275 ("52 no es un objetivo") y las reglas de admisión de `integration-tests/README.md`. La amplitud de cobertura explícitamente no es la meta |
| D-7 | El issue #275 **no se cierra solo con este cambio**; se recomienda una división | Ver "Tratamiento de #275" abajo. Ejecutar la división es una acción separada, fuera de este cambio SDD |
| D-8 | Este cambio es **verificación primero**. Un defecto que un test nuevo exponga se convierte en un seguimiento nombrado, no en alcance absorbido en silencio — **excepto** un arreglo pequeño y localizado, y solo cuando sea necesario para satisfacer exactamente el invariant que PROD-015 está verificando (IS-4 o IS-2) | Una spec de verificación que siempre difiere los arreglos puede terminar demostrando un bug conocido sin cerrar nunca la garantía que vino a verificar. `design.md` DEBE identificar explícitamente el defecto, justificar que la corrección no introduce ninguna capability nueva y delimitar el cambio. Cualquier cosa que requiera una API nueva, comportamiento contractual nuevo, cambio arquitectónico, una migración adicional o una solución no trivial DEBE salir a un follow-up spec atómico separado — sin excepción para eso (decisión del usuario, ronda de preguntas de la propuesta) |
| D-9 | **IS-4 se mantiene en alcance con su peso completo, sin condiciones.** La migration 007 / `aggregate_type_backfill.rs` se trata como una ruta de upgrade soportada mientras el proyecto permita migrar una base desde un estado anterior que la atraviese — sin condicionar su vigencia a que algún deployment conocido ya haya migrado | La corrección de una migración distribuida debe sostenerse para cualquier instalación que la atraviese, sin importar el estado actual de deployments conocidos. Esto evita que `design.md` investigue "estado real de deployments" como condición del alcance de IS-4 — ese estado no determina la corrección de la migración (decisión del usuario, ronda de preguntas de la propuesta) |
| D-10 | **PG14 sigue siendo el piso de compatibilidad real y soportado.** La suite principal (contención, fencing, UoW, concurrencia — IS-1 a IS-6) sigue corriendo sobre PG16 por velocidad. Un slice aparte y acotado de PG14 cubre solo los invariantes sensibles a versión (migraciones, features de SQL/catálogo que podrían divergir genuinamente entre versiones) — no una duplicación completa de la suite | Que el runner hoy provisione PG16 es un hecho de implementación, no una redefinición del mínimo declarado. Si PG14 es el piso declarado, un piso no verificado es una deuda de verificación, no un nuevo mínimo automático. Subir el mínimo efectivo a PG16 debe ser una decisión explícita y separada de producto/versionado, nunca una consecuencia accidental de Testcontainers (decisión del usuario, ronda de preguntas de la propuesta) |

## Puerta de Atomicidad

**Ejecutada, y recortó el alcance dos veces.** Las transiciones de caída/recuperación de la
sonda de readiness (#275 §5) fueron consideradas y descartadas (OOS-2): eso es resiliencia del
pool de conexiones bajo condiciones de red reales, no una garantía de SQL, transacción o
fencing, y la cobertura unitaria de B3.7 ya existe. El límite de `i64::MAX` fue verificado en
lugar de diferido y se eliminó por estar ya cubierto (D-3). Re-verificado tras redactar: cada
ítem restante en alcance nombra un comportamiento de SQL, transacción, bloqueo de fila o
migración de PostgreSQL. No sobrevive en alcance ninguna preocupación de HTTP, socket, OTLP,
readiness, segundo broker, rendimiento general ni CI.

## Alcance

### En Alcance

- **IS-1** — Ejecutar `assert_event_store_conformance` contra `PostgreSQLEventStore` y
  `assert_reservation_store_conformance` contra `PostgresOperationReservationStore`,
  reutilizando **exactamente las mismas definiciones de `ego-testkit`** — nunca una copia
  paralela ni re-derivada. Cierra el AC10 de #275.
- **IS-2** — Concurrencia optimista de la tabla `events`: una violación de unicidad sobre la
  identidad de stream se manifiesta como un conflicto que reporta la versión actual **real**, y
  una carrera de N appends concurrentes sobre un stream deja exactamente un ganador.
- **IS-3** — Seis contendientes compitiendo por un lease expirado: gana exactamente uno, y el
  fencing token avanza **exactamente uno**, no la cantidad de contendientes.
- **IS-4** — Comportamiento transaccional de la migración 007 / `aggregate_type_backfill.rs`:
  abortar antes del primer `UPDATE` deja la tabla idéntica byte a byte; una corrida de cero
  filas commitea; una reversión reincorpora exactamente el estado previo.
- **IS-5** — Semántica de `NULL` para tenant al nivel de identidad de stream de `events`:
  `Option::None` bajo lógica de tres valores (`NULL = NULL` no es verdadero), de forma
  conductual, no desde el catálogo.
- **IS-6** — Atomicidad de la unidad de trabajo — soltar sin commit no persiste nada, una
  unidad de trabajo abierta es invisible para un lector concurrente — satisfecha a través de
  IS-1 salvo que el diseño demuestre lo contrario (D-4).
- **IS-7** — Cada test nuevo se admite bajo las cuatro reglas de admisión de
  `integration-tests/README.md`, declara en su propio doc comment el invariante que prueba y
  **por qué en proceso no puede mostrarse**, y aterriza con su fila en el ledger, su registro
  de módulo y su categoría. El presupuesto end-to-end está gastado (4/4); todo test aquí se
  archiva bajo una categoría no end-to-end con su propio riesgo de infraestructura declarado.
- **IS-8** — Validación por mutación/adversarial para los dos invariantes de mayor criticidad
  (IS-3 fencing, IS-4/IS-6 atomicidad transaccional y de unidad de trabajo): neutralizar el
  mecanismo, confirmar que el test nuevo falla y que la suite existente sigue en verde. El
  método es una decisión de `design.md`; esta propuesta solo exige que ocurra.
- **IS-9** — Un slice acotado de compatibilidad con PG14 (D-10): solo los invariantes
  sensibles a versión — la migración 007 y cualquier feature de SQL/catálogo que pudiera
  divergir genuinamente entre versiones de PostgreSQL — corren contra PG14. La suite
  principal (IS-1 a IS-6) se mantiene en PG16. `design.md` elige el mecanismo (una
  matriz/slice pequeña y separada, no una segunda corrida completa de la suite) y nombra
  exactamente qué tests cubre.

### Fuera de Alcance

- **OOS-1** — Bind de socket real y apagado ordenado (`crates/transport`), round-trip OTLP por
  cable (`crates/infrastructure`) y end-to-end HTTP real de CORE-018
  (`examples/reference-app`). Confirmados genuinamente faltantes en HEAD — no existe ninguno de
  `crates/transport/tests/server.rs`, `crates/infrastructure/tests/otlp_export_roundtrip.rs` ni
  `examples/reference-app/tests/e2e_register.rs`, y `examples/reference-app/tests/http_route.rs`
  usa `tower::oneshot`, sin socket real. No son de PostgreSQL y están clasificados como
  loopback hermético, así que no necesitan ni esta suite ni Testcontainers. **Spec futura,
  PROD-016 solo a nivel de nombre** (D-2).
- **OOS-2** — Pruebas de transición de caída/recuperación de la sonda de readiness (#275 §5,
  mecanismo de reenvío TCP). Decidido explícitamente, no descartado en silencio: es resiliencia
  del pool de conexiones bajo condiciones de red reales y no una garantía de PostgreSQL, B3.7
  ya cubre el nivel unitario, y el presupuesto de reloj de la suite rinde más en IS-1 a IS-5.
- **OOS-3** — El límite de agotamiento del fencing en `i64::MAX` (D-3). Ya cubierto en proceso.
- **OOS-4** — Crear el workspace `integration-tests/`, su runner o su guardia de ledger. Todos
  ya existen y se extienden, no se construyen.
- **OOS-5** — Re-entregar el arreglo del pathspec muerto de `scripts/detect-integration-tests.sh`
  o su self-test de seis mutaciones, `fencing_window_postgres.rs` o
  `schema_index_assertion.rs`. Trabajo previo sobre el que este cambio se apoya.
- **OOS-6** — ~~Superada por IS-9/D-10.~~ Originalmente registrada como seguimiento; el
  usuario decidió que el piso de PG14 es real y se mantiene en alcance, de forma acotada,
  como IS-9. La duplicación completa de la suite principal contra PG14 sigue fuera de
  alcance — solo los invariantes sensibles a versión reciben el segundo slice.
- **OOS-7** — Arreglar defectos de producción que los tests nuevos expongan, más allá de lo que
  `design.md` acepte explícitamente (D-8).
- **OOS-8** — Docker Compose, en cualquier lugar, para cualquier cosa.

## Capacidades

### Capacidades Nuevas

- `real-infrastructure-verification`: qué invariantes DEBEN demostrarse contra PostgreSQL real,
  el contrato de admisión que mantiene chica la suite, y el presupuesto de reloj.

### Capacidades Modificadas

- `event-store`: la obligación del contrato de conformidad del event store se extiende a las
  implementaciones **durables**, no solo a las en memoria; más los requisitos de concurrencia
  optimista de la tabla `events` y de identidad de stream con tenant `NULL` (IS-2, IS-5).
- `idempotent-command-processing`: la obligación de conformidad del store de reservas se
  extiende al adaptador durable, y se enuncia el requisito de avance de fencing con muchos
  contendientes (IS-1, IS-3).

Si la fase de spec encuentra que un requisito existente ya implica alguno de estos, se pliega
dentro de `real-infrastructure-verification` en lugar de fabricar un delta.

## Enfoque

Agregar archivos de test al árbol existente `integration-tests/tests/infrastructure/`,
registrar cada módulo y darle a cada uno su fila en el ledger — de lo contrario la guardia
`tests/ledger.rs` hace fallar la corrida, en milisegundos y antes de provisionar contenedor
alguno.

IS-1 va primero deliberadamente y es lo más barato: ambos harnesses ya encajan con sus
adaptadores durables (D-5), así que es un call site más aislamiento de base de datos por test,
y retira IS-6 como efecto lateral (D-4). IS-2, IS-3 e IS-5 son a nivel de store y no
end-to-end por la misma razón que `fencing_window_postgres.rs`: la evidencia *es* el control
preciso de transacciones concurrentes, algo que HTTP no puede expresar. IS-4 ejercita
`aggregate_type_backfill.rs` directamente contra una base de datos migrada.

Las convenciones existentes de la suite son restricciones, no sugerencias: un PostgreSQL
compartido por corrida, aislamiento por esquema o base de datos por test, migraciones una vez
por corrida, **sin sleeps arbitrarios** — sincronizar sobre una señal o hacer polling de
`pg_locks` / `pg_stat_activity` con un deadline explícito.

Principio rector para spec y tasks: **todo test nuevo debe nombrar el invariante exacto que
prueba y justificar por qué es indemostrable en proceso, por contrato, por conformidad o en
tiempo de compilación.** Un test que no puede responder eso no se admite.

## Tratamiento de #275

**El mapeo criterio por criterio es una obligación de la fase de tasks, deliberadamente no
parafraseado aquí.** La exploración confirmó que #275 tiene 13 criterios formales de
aceptación y que el AC10 (harnesses de conformidad reutilizados contra PostgreSQL real) es la
brecha sustantiva restante, cerrada por IS-1. Se estima que los criterios restantes quedaron
resueltos-obsoletos por PROD-012 / PROD-012A / PROD-002-G11. Esta propuesta no reformula
criterios que no puede citar textualmente; `tasks.md` DEBE hacer la verificación contra el
texto vivo del issue, un criterio por fila, cada uno con el archivo que lo satisface.

División recomendada, solo como recomendación documentada:

1. Marcar en #275 los criterios satisfechos, con enlaces a los archivos que los entregan.
2. Acotar #275 a lo que PROD-015 cierra, o derivar los ítems de transporte confirmados como
   restantes (OOS-1) a un issue nuevo.
3. Una spec futura llamada PROD-016 es dueña de la verificación de HTTP / socket / OTLP.

**Ejecutar esta división queda fuera de este cambio SDD.** PROD-015 no crea, edita ni cierra
ningún issue.

## Áreas Afectadas

| Área | Impacto | Descripción |
|------|---------|-------------|
| `integration-tests/tests/infrastructure/` | Nuevo | ~5–6 archivos de test (IS-1 a IS-5) |
| `integration-tests/tests/infrastructure.rs` | Modificado | Registro de módulo por cada archivo nuevo — sin eso la guardia de ledger falla |
| `integration-tests/README.md` | Modificado | Una fila de Status por test nuevo, en una tabla, con la ruta como code span, bajo una categoría declarada con su propio riesgo de infraestructura; conteos del ledger actualizados |
| `crates/testkit/src/{event_store.rs,reservation_conformance.rs}` | Sin cambios (esperado) | Reutilizados textualmente (D-5). Cualquier cambio aquí es una escalación de diseño, no una edición silenciosa |
| `crates/persistence/src/postgres/aggregate_type_backfill.rs`, `migrations/007_*.sql` | Sin cambios | Ejercitados, no modificados (D-8) |
| `integration-tests/src/lib.rs` | Modificado | Runner extendido para aprovisionar y ser dueño de un segundo contenedor PG14 para el slice de compatibilidad IS-9 |
| `integration-tests/src/main.rs` | Modificado | Runner extendido para aprovisionar y ser dueño de un segundo contenedor PG14 para el slice de compatibilidad IS-9 |
| `crates/transport`, `crates/infrastructure`, `examples/reference-app` | Intactos | OOS-1 |
| `Cargo.toml` raíz, `cargo test --workspace` | Intactos | La suite sigue siendo un workspace independiente; la raíz sigue sin Docker |

## Riesgos

| ID | Riesgo | Probabilidad | Mitigación |
|----|--------|--------------|------------|
| R-1 | El mapeo de criterios se difiere a tasks, así que un criterio podría contarse mal como resuelto-obsoleto | Media | `tasks.md` DEBE hacer la verificación contra el texto textual del issue, con un archivo por criterio. #275 no se cierra con este cambio (D-7), así que un conteo errado no puede cerrar en silencio una brecha real |
| R-2 | Presupuesto de reloj: ≤5 min la suite, ≤1–2 min por porción. IS-1 agrega dos corridas completas de conformidad e IS-3 agrega seis contendientes | Media | Tiempo de compilación y de ejecución reportados por separado, como ya hace el runner. Si una porción excede su presupuesto, el arreglo es un test más chico, nunca un presupuesto más grande |
| R-3 | Los harnesses de conformidad aseguran listados exactos de streams, así que una base compartida los volvería dependientes del orden y de los vecinos | Media | Base de datos aislada por test, que la suite ya provee. Confirmar en `design.md` antes del primer RED |
| R-4 | D-4 asume que `store.load()` en `PostgreSQLEventStore` usa una conexión del pool distinta de la de la unidad de trabajo abierta | Media | Verificado en `design.md` antes de retirar IS-6. Si no se sostiene, IS-6 pasa a ser su propio test con su propia fila de ledger |
| R-5 | Crecimiento de la suite por acumulación — el modo de falla que `integration-tests/README.md` fue escrito para prevenir | Media | D-6 (sin objetivo de cantidad), IS-7 (reglas de admisión por test) y el presupuesto end-to-end 4/4 ya gastado. Las categorías no son un atajo |
| R-6 | IS-4 o IS-2 exponen un defecto real de producción y el cambio absorbe el arreglo | Media | D-8: verificación primero. Un defecto se vuelve seguimiento nombrado salvo que `design.md` acepte explícitamente un arreglo pequeño |
| R-7 | Carga de revisión: ~5–6 archivos de test más la prosa del ledger puede superar el presupuesto de 400 líneas | Media | `tasks.md` lo pronostica y divide en PRs encadenados — IS-1 es una primera porción natural que cierra el AC10 por sí sola |
| R-8 | `docs/integration-test-backlog.md` y `skills/testing-strategy/SKILL.md` están ambos obsoletos en HEAD (el segundo afirma que existen tres archivos de test de transporte; ninguno existe) | Baja | Registrado aquí para que ninguna fase posterior los trate como fuente de verdad. Corregirlos no está en alcance |
| R-9 | IS-9 (slice de PG14) agrega un segundo target de versión de PostgreSQL a la suite, exactamente el patrón de acumulación contra el que R-5 protege si se acota de forma laxa | Media | Acotado explícitamente por D-10/IS-9 solo a invariantes sensibles a versión — migración 007 y features de catálogo/SQL que podrían divergir genuinamente. `design.md` nombra el conjunto exacto de tests; nunca es "correr todo dos veces" |

## Plan de Reversión

Solo tests y aditivo. Revertir es borrar los archivos nuevos bajo
`integration-tests/tests/infrastructure/`, sus registros de módulo y sus filas de ledger en el
README — la guardia de ledger verifica que los tres se mantengan consistentes en ambas
direcciones, así que una reversión parcial falla ruidosamente en lugar de en silencio. No se
toca código de producción, ni esquema, ni migración, ni datos, ni API pública, así que nada
fuera de `integration-tests/` puede regresionar. Si un cambio en `crates/testkit` resultara
inevitable (D-5 dice que no debería), es aditivo y se revierte junto con su call site.

## Dependencias

- El workspace `integration-tests/` existente, su runner
  (`cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite`), su contenedor
  compartido y su guardia `tests/ledger.rs`.
- Los harnesses de conformidad de `ego-testkit` existentes — reutilizados, no modificados.
- Los adaptadores durables existentes: `PostgreSQLEventStore`,
  `PostgresOperationReservationStore`, `aggregate_type_backfill.rs` y la migración 007.
- Un daemon de Docker alcanzable para la suite. Ningún crate, servicio ni dependencia externa
  nueva.

## Criterios de Éxito

- [ ] **SC-1** — `assert_event_store_conformance` y `assert_reservation_store_conformance`
      corren ambos contra sus adaptadores durables de PostgreSQL, usando las mismas
      definiciones de `ego-testkit` que usan los llamadores en memoria. El AC10 de #275 queda
      cerrado.
- [ ] **SC-2** — Una versión esperada obsoleta en la tabla `events` produce un conflicto que
      reporta la versión actual real, y una carrera de N appends sobre un stream deja
      exactamente un ganador.
- [ ] **SC-3** — Seis contendientes compitiendo por un lease expirado producen exactamente un
      ganador y un fencing token que avanzó exactamente uno.
- [ ] **SC-4** — El backfill de la migración 007 queda probado como transaccional: abortar
      antes del primer `UPDATE` deja la tabla idéntica byte a byte, una corrida de cero filas
      commitea, y una reversión reincorpora exactamente el estado previo.
- [ ] **SC-5** — La semántica de tenant `NULL` en la identidad de stream de `events` se asegura
      de forma conductual, no desde el catálogo.
- [ ] **SC-6** — La atomicidad de la unidad de trabajo se sostiene contra PostgreSQL real, sea
      a través de IS-1 (D-4) o mediante su propio test si el diseño lo exige.
- [ ] **SC-7** — Cada test nuevo declara su invariante y su justificación de por-qué-no-en-proceso
      en su propio doc comment, y `tests/ledger.rs` pasa sin deriva.
- [ ] **SC-8** — Queda registrada la verificación por mutación para IS-3 e IS-4/IS-6: con el
      mecanismo neutralizado el test nuevo falla. Para IS-4/IS-6 (migración 007 / atomicidad de
      unidad de trabajo), el resto de la suite preexistente sigue en verde. Para IS-3
      (fencing), como el test nuevo de fencing con muchos contendientes comparte su predicado
      portante con el test preexistente de fencing con un solo contendiente
      (`fencing_window_postgres.rs`), neutralizar ese predicado hace fallar tanto al test nuevo
      como a ese test preexistente — el verdor global de la suite no es la afirmación; todo
      test que no ejercite el predicado compartido permanece sin afectar.
- [ ] **SC-9** — La suite completa termina en ≤5 minutos, ninguna porción excede 1–2 minutos, y
      el tiempo de compilación se reporta por separado del de ejecución.
- [ ] **SC-10** — `cargo test --workspace` sigue sin Docker y sin cambios; el workspace raíz
      queda intacto.
- [ ] **SC-11** — No aparece en el cambio entregado ningún trabajo de HTTP, socket, OTLP, sonda
      de readiness ni CI, y queda registrada la recomendación de PROD-016 para la división.
- [ ] **SC-12** — `tasks.md` contiene la verificación criterio por criterio contra el texto
      textual de #275, un criterio por fila con el archivo que lo satisface.
- [ ] **SC-13** — Los invariantes sensibles a versión (migración 007, features de
      catálogo/SQL que podrían divergir) quedan probados contra PG14 a través de un slice
      acotado y separado — no una segunda corrida completa de la suite — mientras la suite
      principal (IS-1 a IS-6) se mantiene en PG16.
