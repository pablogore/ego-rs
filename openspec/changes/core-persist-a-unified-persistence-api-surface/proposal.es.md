# Propuesta: CORE-PERSIST-A — Superficie Unificada de API de Persistencia (Puertos Propiedad del Dominio)

> Compañero de revisión en español. Fuente de verdad canónica: `proposal.md` (identificadores 1:1).

## Objetivo

Dar a cada puerto de persistencia propiedad del dominio un único crate propietario. Hoy el
vocabulario de puertos de persistencia está disperso entre `ego_domain::persistence::*`,
`ego_domain::read_side::*` y `ego_domain::operation::*`, sin ninguna frontera de crate que
distinga un puerto del resto del modelo de dominio. CORE-PERSIST-A reubica ese vocabulario
dentro de un nuevo crate `ego-persistence-api`, con `ego-domain` reexportando cada
elemento en su ruta actual exacta, de modo que ningún consumidor fuera de esos dos crates cambie
una sola línea.

**Enmendado tras `sdd-design`**: "ese vocabulario" es más amplio de lo que esta propuesta
calculó originalmente. `design.md` encontró que los puertos movidos no compilan de forma
aislada — `EventTag`, `ProjectionState`, `EventStreamElement` y `DomainEvent` son arrastrados
por las propias firmas de los puertos, y el macro `id_type!` que genera `TenantId` se comparte
con cuatro tipos de identidad de dominio no relacionados. Ver la sección resuelta
**OD-1 / OQ-1 / OQ-2** abajo y AD-2/AD-3 de design.md para el cierre completo y su razonamiento.

## Intención

**El problema es de propiedad, no de comportamiento.** Un puerto no tiene hogar. `EventStore`
está junto a `AggregateRoot`; `OffsetStore` está junto a `EventTag`; un lector no puede deducir
del árbol de módulos cuál de ellos está obligado a implementar un adaptador de persistencia.
El hallazgo 5 del §10 de la exploración muestra dónde termina eso:
`crates/runtime/src/effects/store.rs` define tres puertos, todo su vocabulario de tipos de
contrato y una implementación en memoria funcional, en un solo archivo de 1320 líneas, dentro de
un crate que ni siquiera es `ego-domain`.

**Este cambio es puramente estructural.** Sin SQL, sin firmas, sin cambios async/sync, sin
object-safety, sin comportamiento en ejecución. Cada elemento reubicado se mueve textualmente
—comentarios de documentación y pruebas unitarias colocadas incluidas— y se reexporta en su ruta
antigua. Lo único observable para un consumidor es que los mismos elementos ahora también
resuelven bajo `ego_persistence_api::*`.

**Toma exactamente una decisión de arquitectura** (D-2): `ego-domain` puede depender de
`ego-persistence-api`. Esa decisión no es una comodidad —es forzada, y la evidencia está en
§D-2. Nombrarla aquí es la razón de ejecutar esta rebanada por separado.

## Decisiones Activas

| ID | Decisión | Justificación |
|----|----------|---------------|
| D-1 | **Nuevo crate `ego-persistence-api` en `crates/persistence-api/`, mapeado a la capa `domain` en `layers.toml`** | Coincide con la convención de paquetes `ego-*` y se mantiene distinto del `ego-persistence` existente (adaptadores Postgres). Los puertos son artefactos de dominio en arquitectura hexagonal, así que `domain` es la capa honesta. Los renombres `persistence-postgres` / `persistence-memory` son trabajo de CORE-PERSIST-B/C, no de este cambio |
| D-2 | **La arista de dependencia `ego-domain → ego-persistence-api` es forzada, no elegida.** Código de dominio que se queda ya consume los puertos movidos: `crates/domain/src/read_side/scheduler.rs:5-10` importa `DedupStore`, `OffsetStore` y `ReadSideStore`, y `session.rs`/`runner.rs` hacen lo mismo | Es una dependencia real de compilación, independiente del requisito de reexportación. La arista inversa es por tanto imposible: Cargo prohíbe de plano las dependencias circulares entre crates, y FR-003 las prohíbe otra vez. La dirección la fija el código, no la preferencia |
| D-3 | **D-2 tiene una consecuencia sin resolver, y esta propuesta la nombra en lugar de asumirla resuelta: `ego-persistence-api` NO DEBE depender de `ego-domain`, pero tres elementos movidos referencian tipos de dominio que se quedan atrás** — `DomainEvent` (`persistence/event_store.rs:3,47,186`) y `TenantId` (`operation/receipt.rs:11`, `operation/reservation.rs:32`) | El §5 de la exploración lo trató como condicional ("solo si algún puerto movido necesita un tipo de dominio"). No es condicional; está confirmado. **`design.md` DEBE resolverlo antes de mover código alguno.** Las rutas candidatas están bajo **Decisión Abierta OD-1**. Elegir entre ellas es una decisión de diseño con compromisos reales, no un trámite mecánico |
| D-4 | **La compuerta `foundation-integrity` debe relajarse para admitir una arista de misma capa `domain → domain`.** `xtask/src/layers.rs:76` dice `"domain" => Some(&[])` — un crate `domain` puede depender de **nada**, ni siquiera de otro crate `domain`. Cualquier arista nueva de `ego-domain` falla FR-002 hoy | La relajación es la más estrecha disponible: `Some(&[])` → `Some(&["domain"])`, igualando la auto-arista que la matriz ya concede a `foundation` (`["domain","foundation"]`) y a `infrastructure`. Mapear el crate nuevo a `foundation` sería un agujero mucho más ancho: legalizaría `ego-domain → ego-runtime`. FR-003 y Cargo siguen bloqueando el ciclo, así que la relajación no puede degenerar en una inversión |
| D-5 | **Cada elemento reubicado se reexporta en su ruta antigua exacta.** Cero sentencias `use` cambian fuera de los dos crates; cero archivos `Cargo.toml` ganan una arista fuera de `ego-domain` y `ego-persistence-api` | El §6 de la exploración contó 92 archivos que resuelven estos elementos por ruta de módulo exacta. La reexportación es lo que hace este cambio reversible en pleno vuelo (ver Plan de Reversión) y lo que lo mantiene revisable |
| D-6 | **La reubicación es textual.** Comentarios de documentación, módulos `#[cfg(test)]` e impls generales se mueven con su elemento y no se reescriben, reorganizan ni "mejoran" | Las impls generales de reenvío `Arc<T>` de `OffsetStore`/`DedupStore` (`offset.rs:92`, `dedup.rs:60`) son críticas: perder un reenvío reclasifica en silencio todo par durable registrado como volátil (§7 de la exploración). El movimiento textual es la única forma de garantizar cero deriva semántica en un diff tan amplio |
| D-7 | **La matriz del §9 de la exploración es un piso, no un techo.** Cada elemento de un módulo reubicado se mueve, incluidos los que el §9 omite (`OperationId`, `OwnerId`, `StoredServiceResponse`, `OperationKeyError`, `OperationKeyHash`, `AggregateOutcomeError`, `ProjectionStateStoreError`, listados en el §3 pero ausentes del §9) | Un módulo movido a medias no compila. El §9 además discrepa del §11 en el conteo: el §11 dice "filas 1–26", el conteo directo de las filas propiedad del dominio da **27**. La matriz manda sobre el resumen; el conjunto exacto se fija durante `sdd-design` |
| D-8 | **`ProjectionStateStore` se reubica tal cual**, marcado como deuda conocida, no eliminado | Hallazgo 3 del §10: cero implementaciones, cero consumidores. Eliminarlo convertiría este cambio en un cambio de comportamiento disfrazado de reorganización. La deuda se nombra (KD-1), no se limpia en silencio |
| D-9 | **Los tres puertos de effect-store propiedad de `ego-runtime` quedan completamente diferidos.** `ego-runtime` y `ego-effect-store` no se tocan | §11 de la exploración: reubicarlos deja o bien a `InMemoryEffectStore` dependiendo de un puerto definido en otra parte (una decisión de dirección de dependencia *nueva*), o bien exige mover una implementación (prohibido aquí). Esa es una segunda decisión de arquitectura y pertenece a su propio cambio (F-1) |

## Decisiones Abiertas — resueltas por `design.md`, confirmadas por el dueño del cambio

**OD-1 — resuelta: relajar la compuerta de capas, no agregar un crate, no redimensionar.**
`ego-persistence-api` alcanza `DomainEvent`/`TenantId` manteniéndose como crate hoja y dejando
que `ego-domain` dependa de él (D-2), con `allowed_layers("domain")` relajado de `Some(&[])` a
`Some(&["domain"])` (D-4). Las otras dos rutas de las cuatro originales —un tercer crate hoja
para los tipos compartidos, y redimensionar A1 dejando atrás `EventStore`/`Repository`— se
rechazaron por ser, respectivamente, un segundo crate nuevo que el presupuesto de la rebanada no
puede absorber, y una partición que derrota el objetivo. AD-1 de design.md tiene el registro
completo.

**OQ-1 — resuelta: aceptar el closure ampliado.** El diseño encontró que el closure de
compilación son cinco tipos, no los dos que nombraba D-3: `EventTag`, `ProjectionState`,
`EventStreamElement` (ya vocabulario de puertos read-side — `OffsetStore`/`DedupStore`/
`ReadSideStore`/`ProjectionStateStore` están indexados por ellos o los producen), más
`DomainEvent` mismo (el único elemento cuyo hogar este cambio empeora, movido solo porque
`EventStore<E: DomainEvent>` no compila sin él). Los cuatro se reubican textualmente,
reexportados en sus rutas antiguas, igual que cualquier otro elemento. No se viola ningún OOS —
sin cambio de firma, sin tipo nuevo, sin mover implementaciones. AD-2 de design.md tiene el
registro completo.

**OQ-2 — resuelta: reubicar el macro `id_type!`, un solo generador.** El generador de
`TenantId` (`context.rs:7-54`) se comparte con cuatro tipos de identidad de dominio no
relacionados (`AggregateId`, `EntityId`, `CorrelationId`, `CausationId`, `RequestId`). El macro
se muda a `ego-persistence-api` como `#[macro_export]`; `ego-domain` invoca el macro
reexportado (vía la arista de OD-1) para seguir generando los otros cinco tipos localmente.
Duplicar el macro en su lugar fue rechazado — es la única ruta no textual disponible y viola
D-6. AD-3 de design.md tiene el registro completo.

## Compuerta de Atomicidad

**Ejecutada, y ya recortó el alcance una vez.** CORE-PERSIST-A originalmente agrupaba los puertos
de effect-store de `ego-runtime`; el §11 de la exploración encontró que eso agrupaba dos
decisiones de arquitectura independientes, y se tomó la división (D-9 → F-1).

Lo que queda es un movimiento indivisible. Los elementos reubicados (35 según la enumeración
directa de EC-4 en design.md, que reemplaza este conteo) comparten un crate
destino, una decisión de dirección de dependencia (D-2), una relajación de compuerta (D-4) y una
capa de reexportación (D-5). Reubicar un subconjunto dejaría el vocabulario de puertos partido
entre dos crates, que es exactamente la condición que este cambio existe para terminar.

**ATOMICIDAD: PASA** — con la salvedad de que OD-1 es una pregunta *de diseño* abierta dentro de
un alcance atómico, no un segundo cambio escondido.

## Alcance

**Frontera de un vistazo**

| | |
|---|---|
| **CORE-PERSIST-A incluye** | Nuevo crate `ego-persistence-api` · 35 puertos y tipos de contrato propiedad del dominio reubicados textualmente (design.md EC-4) más los añadidos del closure de OQ-1/OQ-2 arriba · reexportaciones en cada ruta antigua · entrada en `layers.toml` · relajación de dirección de `foundation-integrity` |
| **CORE-PERSIST-A excluye** | Toda implementación · `ego-runtime` / `ego-effect-store` · todo cambio de SQL, migración, firma o comportamiento · todo elemento de deuda conocida |

### En Alcance

- **IS-1** — Un nuevo crate `ego-persistence-api` en `crates/persistence-api/`, nombre de paquete
  `ego-persistence-api`, con la disposición del §8 de la exploración menos el subárbol `effects/`
  (D-1, D-9).
- **IS-2** — Reubicación textual de todo puerto y tipo de contrato propiedad del dominio desde
  `crates/domain/src/{persistence,read_side/{offset,dedup,store,projection_state_store},operation/{reservation,key,receipt}}`
  hacia ese crate (D-6, D-7). **Enmendado por OQ-1**: también incluye
  `read_side/{event_tag,state,event_stream}.rs` y `event.rs` (`DomainEvent`), forzados hacia el
  crate por las propias firmas de los puertos movidos — ver la sección de Decisiones Abiertas
  resuelta arriba.
- **IS-2b** (nuevo, OQ-2) — El macro `id_type!` (`context.rs:7-54`) se reubica textualmente y
  gana `#[macro_export]`; `TenantId`/`TenantIdError` se generan en `ego-persistence-api`.
  `ego-domain` reinvoca el macro para sus otros cuatro tipos de identidad y reexporta
  `TenantId`/`TenantIdError` en sus rutas existentes `ego_domain::context::*` / `ego_domain::*`.
- **IS-3** — Una reexportación `pub use` en `ego-domain` en la ruta actual exacta de cada elemento
  reubicado (D-5), incluidas las reexportaciones de raíz de crate que `ego-domain` ya publica.
- **IS-4** — Los consumidores internos del propio `ego-domain` (`read_side/scheduler.rs`,
  `session.rs`, `runner.rs`) recableados al crate nuevo, más la única arista nueva de
  `Cargo.toml` (D-2).
- **IS-5** — Una entrada en `layers.toml` que mapee `ego-persistence-api` a `domain` (FR-001), y la
  relajación de `allowed_layers("domain")` de `Some(&[])` a `Some(&["domain"])` en
  `xtask/src/layers.rs:76` con su prueba unitaria de cobertura (D-4).
- **IS-6** — Una prueba de compilación de que cada ruta antigua sigue resolviendo al mismo
  elemento — no a una copia redeclarada con el mismo nombre.
- **IS-7** — Deltas de especificación según la sección Capacidades.

### Fuera de Alcance

Cada elemento de abajo es un **no-objetivo**, no un olvido. Varios son deuda nombrada con dueño.

- **OOS-1 — Ninguna implementación se mueve.** `InMemoryEventStore`, `InMemoryRepository`,
  `InMemorySnapshotStore`, `InMemoryReadSideStore`, `InMemoryOperationReservationStore` y todo
  adaptador `PostgreSQL*`/`Postgres*` se quedan exactamente donde están. Solo se mueven traits de
  puerto y tipos de contrato.
- **OOS-2 — `ego-runtime` y `ego-effect-store` no se tocan en absoluto.** `EffectStateStore`,
  `EffectDedupStore` y `RetentionMaintenance` se quedan en `crates/runtime/src/effects/store.rs`
  (D-9 → F-1).
- **OOS-3 — Ningún cambio de SQL, migración, índice, transacción, reintento, clasificación de
  errores, pool de conexiones o durabilidad, de ningún tipo.**
- **OOS-4 — Ningún cambio de firma de método, async/sync, `Send`/`Sync` u object-safety.** La
  forma de un trait tras este cambio debe ser idéntica byte a byte a su forma previa, salvo por la
  ruta de módulo.
- **OOS-5 — Ningún cambio de semántica de tenant.** La regla de tres vías de `resolve_tenant`
  (`None` / `Some("")` / `Some(t)`) se mueve textualmente y no se revisa.
- **OOS-6 — Ningún cambio de comportamiento en producción.** Nada de este cambio es observable en
  tiempo de ejecución.
- **OOS-7 — Ninguna fusión de crates.** Ningún crate existente se pliega dentro de otro.
- **OOS-8 — Ninguna capacidad nueva.** No se agrega ni un trait, método o tipo que no exista hoy.
- **OOS-9 — `ProjectionStateStore` no se elimina** (D-8 → KD-1).
- **OOS-10 — Ninguna abstracción SQL genérica, motor de dialectos, constructor de consultas ni
  construcción con forma de ORM.**
- **OOS-11 — Ningún trabajo de Oracle, MySQL ni cualquier otro backend.**
- **OOS-12 — El defecto confirmado de `crates/persistence/src/postgres/repository.rs` no se
  corrige aquí** (KD-2).
- **OOS-13 — `crates/persistent-entity/src/types.rs` no se elimina ni se cablea** (KD-3).
- **OOS-14 — No se agrega ninguna suite de conformidad** para las capacidades que carecen de ella
  (`Repository`, `Snapshot`, `OffsetStore`, `DedupStore`) — hallazgo 4 del §10, propiedad de
  CORE-PERSIST-D/E.

## Capacidades

### Capacidades Nuevas

- `persistence-api-surface`: el contrato observable de que el vocabulario de puertos de
  persistencia propiedad del dominio tiene exactamente un crate propietario, y de que cada ruta
  que un consumidor resuelve hoy sigue resolviendo al mismo elemento.

### Capacidades Modificadas

- `foundation-integrity`: la regla de dirección de FR-002 permite hoy a un crate `domain` cero
  dependencias. Debe admitir una arista de misma capa `domain → domain`, igualando la auto-arista
  que la matriz ya concede a `foundation` y a `infrastructure` (D-4).

Si la fase de especificación encuentra que un requisito existente ya implica alguna de estas, la
pliega en lugar de fabricar un delta.

## Enfoque

Crear el crate, mover textualmente el archivo de cada módulo, reemplazar el módulo vaciado de
`ego-domain` por una reexportación `pub use ego_persistence_api::…::*;` en la ruta idéntica, y
agregar la única arista de `Cargo.toml`. Recablear solo los consumidores internos de puertos del
propio `ego-domain`. Nada fuera de los dos crates se edita: ni una sentencia `use`, ni un
`Cargo.toml`, ni una prueba.

El orden importa para la revisabilidad: resolver OD-1 primero (puede cambiar qué significa
"textual" para `EventStore` y `OperationReceipt`), luego mover, luego reexportar, luego relajar la
compuerta. La relajación de la compuerta aterriza junto con la arista que la necesita, nunca
antes.

## Deuda Conocida (arrastrada, no corregida)

Cada elemento queda registrado para que tenga un dueño nombrado y no uno implícito.

- **KD-1 — `ProjectionStateStore` está muerto.** Cero implementaciones, cero consumidores en todo
  el workspace (hallazgo 3 del §10). Se reubica tal cual (D-8). Su eliminación pertenece al cambio
  que decida el conjunto de puertos del lado de lectura.
- **KD-2 — `crates/persistence/src/postgres/repository.rs` carga un defecto confirmado de dos
  partes.** Las líneas 82, 135 y 161 usan `tenant_id = $2` donde todos los adaptadores hermanos
  del mismo crate usan correctamente `IS NOT DISTINCT FROM $2`, por lo que la partición de tenant
  systemwide (`NULL`) se maneja mal. Peor aún, el `INSERT … ON CONFLICT (aggregate_id, tenant_id)`
  de la línea 109 apunta a una restricción que **no existe** —la migración
  `002_create_aggregates.sql` declara `aggregate_id VARCHAR(255) PRIMARY KEY` a secas—, lo que
  Postgres rechaza con `42P10`. Es un riesgo vivo de fallo en ejecución y de aislamiento de
  tenant, no cosmético. **No se corrige aquí** (OOS-12), y no debería esperar a la serie
  CORE-PERSIST: F-2.
- **KD-3 — `crates/persistent-entity/src/types.rs` es código muerto con una duplicación interna.**
  Nunca referenciado por una declaración `mod`, y se auto-duplica `EntityTriple`, `EntityId` y
  `ExecutionKey` (líneas 18/122, 52/143, 85/168) — un `E0428` duro si alguna vez se cableara.
  Además define el alias `TenantId = String`, que colisiona por nombre con el newtype validado de
  `ego-domain`. No se elimina (OOS-13): F-3.
- **KD-4 — La cobertura de conformidad es asimétrica.** `Repository`, `Snapshot`, `OffsetStore` y
  `DedupStore` no tienen suite de conformidad en ninguna parte, pese a que el valor por defecto de
  `is_durable()` es una trampa documentada. Propiedad de CORE-PERSIST-D/E (OOS-14).

## Semántica Requerida

```
Dado cualquier crate que hoy compila `use ego_domain::persistence::EventStore;`
Cuando se compile tras este cambio con esa sentencia sin editar
Entonces DEBE compilar, y el elemento resuelto DEBE ser el mismo trait — no una
        copia redeclarada que solo comparte el nombre.

Dado que lo mismo vale para cada ruta de las filas propiedad del dominio del §9
Cuando se construya el workspace
Entonces ningún crate fuera de ego-domain y ego-persistence-api tiene una
        sentencia `use` editada ni una dependencia agregada en Cargo.toml.

Dado un trait reubicado por este cambio
Cuando se compare su definición posterior con su definición previa
Entonces cada firma de método, cota, supertrait y cuerpo por defecto DEBE ser
        idéntico, difiriendo solo en la ruta de módulo.

Dadas las impls generales de reenvío Arc<T> de OffsetStore y DedupStore
Cuando se registre un almacén detrás de un Arc tras este cambio
Entonces is_durable() DEBE seguir reenviando al almacén interno, y un par
        durable NO DEBE reclasificarse como volátil.

Dada la compuerta foundation-integrity
Cuando se ejecute sobre el workspace posterior al cambio
Entonces DEBE pasar: ego-persistence-api está mapeado, la arista de ego-domain
        está permitida, no existe ciclo, y cada crate compila en aislamiento.

Dada la suite de pruebas del workspace
Cuando se ejecute `cargo test --workspace` tras este cambio
Entonces DEBE mostrar cero fallos nuevos y cero aserciones modificadas.
```

## Áreas Afectadas

| Área | Impacto | Descripción |
|------|---------|-------------|
| `crates/persistence-api/` | Nuevo | El crate completo: `Cargo.toml` + módulos reubicados (IS-1, IS-2) |
| `crates/domain/src/{persistence,read_side,operation}/` | Modificado | Definiciones reemplazadas por reexportaciones en rutas idénticas (IS-3) |
| `crates/domain/src/read_side/{scheduler,session,runner}.rs` | Modificado | Importaciones recableadas al crate nuevo (IS-4) |
| `crates/domain/Cargo.toml` | Modificado | Una arista nueva (D-2) |
| `layers.toml` | Modificado | Una entrada: `"ego-persistence-api" = "domain"` (IS-5) |
| `xtask/src/layers.rs:76` | Modificado | Relajación de `allowed_layers("domain")` + prueba de cobertura (D-4, IS-5) |
| `Cargo.toml` (miembros del workspace) | Modificado | Miembro nuevo |
| `crates/{infrastructure,persistence,runtime,testkit,service-sdk,persistent-entity}`, `examples/reference-app`, `integration-tests` | Sin tocar | Protegidos por las reexportaciones (D-5) |
| `crates/runtime/src/effects/store.rs`, `crates/effect-store/` | Sin tocar | Diferido (OOS-2) |
| `openspec/specs/{persistence-api-surface,foundation-integrity}/spec.md` | Nuevo / Modificado | Deltas según IS-7 |

## Riesgos

| ID | Riesgo | Probabilidad | Mitigación |
|----|--------|--------------|------------|
| R-1 | **OD-1 no tiene respuesta que quepa en las prohibiciones de esta rebanada**, y `apply` lo descubre a mitad del movimiento | Alta | OD-1 es una decisión bloqueante nombrada para `design.md`, explícitamente previa a cualquier movimiento de código (Enfoque). Si el diseño no puede cerrarla, el resultado correcto es redimensionar, no romper OOS-4 u OOS-8 en silencio |
| R-2 | Un elemento reubicado deriva —una cota perdida, un cuerpo por defecto alterado, una impl general extraviada— dentro de un diff demasiado grande para leer línea a línea | Media | D-6 convierte la reubicación textual en regla, no en preferencia. Las impls de reenvío `Arc<T>` tienen su propia cláusula de Semántica Requerida y SC-4, porque perder una es silencioso (§7) |
| R-3 | La relajación de `foundation-integrity` (D-4) se lee después como "los crates de dominio pueden depender hacia abajo", y una inversión genuina se cuela | Media | La relajación es la más estrecha disponible: solo misma capa. FR-003 y Cargo siguen prohibiendo el ciclo, así que `ego-persistence-api → ego-domain` sigue siendo imposible por construcción, no por vigilancia de revisión. SC-7 afirma que la matriz admite `domain → domain` y nada más ancho |
| R-4 | **Presupuesto de revisión.** El §11 estima 1.500–2.000 líneas reubicadas contra un presupuesto de 400 | Alta | Aceptado y pronosticado, no escondido. `sdd-tasks` debe rebanar esto en PRs encadenados por grupo de capacidad (`persistence/`, `read_side/`, `operation/`), cada uno con la capa de reexportación intacta para que toda rebanada intermedia compile en todo el workspace. El diseño de reexportación (D-5) es lo que hace seguro el encadenamiento |
| R-5 | El conjunto exacto a mover es incorrecto — la matriz del §9 omite elementos que el §3 lista, y el conteo del §11 discrepa del §9 | Media | Nombrado de frente (D-7). El conjunto autoritativo es "todo elemento público de un módulo reubicado", fijado durante `sdd-design` contra el código fuente, nunca contra el resumen |
| R-6 | Se omite una reexportación de una ruta que ninguna prueba ejercita, rompiendo a un consumidor aguas abajo solo en su propia compilación | Media | IS-6 exige una prueba de compilación sobre la lista completa de rutas, no verificaciones puntuales. `cargo build --workspace` más la compilación en aislamiento de FR-005 cubren a los consumidores dentro del árbol |

## Seguimientos Nombrados (deliberadamente no plegados)

- **F-1 — CORE-PERSIST-A2 (o plegado en CORE-PERSIST-B) — reubicar los puertos de effect-store
  propiedad de `ego-runtime`.** `EffectStateStore`, `EffectDedupStore`, `RetentionMaintenance` y su
  vocabulario de tipos de contrato. El cambio debe decidir explícitamente si `ego-runtime`
  conserva `InMemoryEffectStore` y gana una dependencia nueva, o si la implementación de
  conveniencia también se mueve. También le toca preguntar si `ego-effect-store → ego-runtime`
  sobrevive siquiera, una vez desaparecida su razón declarada de existir (D-9, OOS-2).
- **F-2 — Corregir el alcance de tenant y el objetivo de `ON CONFLICT` de `PostgreSQLRepository`**
  (KD-2). **Esto no debería esperar a la serie CORE-PERSIST.** La exposición a `42P10` es una ruta
  viva de fallo en ejecución y la mitad de alcance de tenant es un defecto de aislamiento; ambas
  ameritan una corrección independiente con sus propias pruebas, agendada por separado.
- **F-3 — Eliminar o reparar `crates/persistent-entity/src/types.rs`** (KD-3).
- **F-4 — CORE-PERSIST-D/E — suites de conformidad** para `Repository`, `Snapshot`, `OffsetStore` y
  `DedupStore`, y el eventual hogar `persistence-testkit` (KD-4).

## Plan de Reversión

**Un solo commit de reversión, en cualquier momento, con cero rotura externa.**

Como cada elemento reubicado se reexporta en su ruta antigua exacta (D-5), ningún crate fuera de
`ego-domain` y `ego-persistence-api` depende jamás de la disposición nueva. Revertir es por tanto:
eliminar `crates/persistence-api/`, restaurar los módulos de `ego-domain` desde el árbol previo,
quitar la única arista de `Cargo.toml`, quitar la entrada de `layers.toml` y restaurar
`allowed_layers("domain")` a `Some(&[])`. Nada más se toca en ninguna de las dos direcciones.

Esto vale **en pleno vuelo** también, que es lo que hace seguro el rebanado en PRs encadenados de
R-4: cada rebanada mantiene intacta la capa de reexportación, de modo que un CORE-PERSIST-A
aterrizado parcialmente es un workspace donde algunos puertos viven en un segundo crate y todo
consumidor sigue compilando sin cambios. No existe estado intermedio que exija una reversión
coordinada entre varios crates.

Ningún dato, esquema, migración ni estado persistido está involucrado en ninguna dirección — este
cambio no escribe nada en tiempo de ejecución.

## Dependencias

- `foundation-integrity` (archivado) — FR-001, FR-002, FR-003, FR-005 y la compuerta
  `xtask verify-layers`. FR-002 es modificado por este cambio; el resto se consume sin cambios.
- La regla de diseño de `openspec/config.yaml` "Sin dependencias circulares entre crates" —
  sostenida por construcción (D-2).
- El artefacto de exploración `explore.md` §9, matriz de movimiento/reexportación, con la
  corrección de D-7 aplicada.
- Ninguna dependencia externa, crate, servicio ni infraestructura nueva.

## Criterios de Éxito

- [ ] **SC-1** — Toda ruta propiedad del dominio de la matriz del §9 sigue resolviendo, sin editar,
      al mismo elemento. Probado en tiempo de compilación sobre la lista completa (IS-6), no por
      muestreo.
- [ ] **SC-2** — Ningún crate fuera de `ego-domain` y `ego-persistence-api` tiene una sentencia
      `use` editada ni una dependencia agregada en `Cargo.toml`. Verificable solo desde el diff.
- [ ] **SC-3** — Las firmas de método, cotas, supertraits y cuerpos por defecto de cada trait
      reubicado son idénticos a su texto previo, difiriendo solo en la ruta de módulo (OOS-4).
- [ ] **SC-4** — Las impls generales de reenvío `Arc<T>` de `OffsetStore` y `DedupStore` se
      movieron intactas; `is_durable()` sigue reenviando y ningún par durable se reclasifica como
      volátil.
- [ ] **SC-5** — `cargo build --workspace` y `cargo test --workspace` pasan con cero fallos nuevos
      y cero aserciones modificadas.
- [ ] **SC-6** — `cargo run -p xtask -- verify-layers` pasa: `ego-persistence-api` está mapeado
      (FR-001), la arista de `ego-domain` está permitida (FR-002), no existe ciclo (FR-003) y el
      crate nuevo compila en aislamiento (FR-005).
- [ ] **SC-7** — La matriz de dirección admite `domain → domain` y nada más ancho. Una prueba
      afirma que `domain → foundation`, `domain → infrastructure` y `domain → sdk` siguen fallando.
- [ ] **SC-8** — Cero archivos de SQL, migración o esquema aparecen en el diff (OOS-3).
- [ ] **SC-9** — `crates/runtime/`, `crates/effect-store/` y toda estructura de implementación
      nombrada en OOS-1 quedan sin modificar (OOS-1, OOS-2).
- [ ] **SC-10** — OD-1 queda cerrada en `design.md` con decisión y justificación declaradas antes
      de mover código alguno, y la ruta elegida no viola ninguna de OOS-1 a OOS-14.
- [ ] **SC-11** — KD-1 a KD-4 aparecen como deuda nombrada con dueños de seguimiento nombrados, y
      ninguna de ellas se corrige, elimina ni aborda parcialmente en este cambio.
- [ ] **SC-12** — `sdd-tasks` produce un plan de PRs encadenados donde cada rebanada compila en
      todo el workspace por sí sola, con la capa de reexportación intacta en cada paso (R-4).
