# Diseño: STOOLAP-S1 — Adaptador `Repository` de primera clase sobre Stoolap

> Compañero de revisión en español. La fuente canónica es `design.md` (identificadores 1:1:
> EC-1..EC-7, AD-1..AD-11, S1..S3, OQ-1..OQ-3).
>
> **Entradas**: `proposal.md` (D-1..D-12, NG-1..NG-9, IS-1..IS-6, R1..R14, KD-1..KD-3, F-1..F-4,
> RK-1..RK-7) y la exploración STOOLAP-S1 (Engram `sdd/stoolap-s1/explore`, veredicto **GREEN**).
> Este documento decide el **cómo**: el esquema y su codificación de tenant, el DSN de durabilidad,
> la forma exacta de las sentencias y la lógica de mapeo de conflictos, el cierre de dependencias y
> el árbol de módulos del crate, la firma y el conjunto de escenarios del arnés de conformidad
> compartido, y los límites de los cortes. Los requisitos observables pertenecen a `spec.md` y no se
> repiten aquí.
>
> **Línea base leída**: `develop` @ `e2bf2b4`. Cada `archivo:línea` de abajo se leyó sobre esa línea
> base, y cada cita de `stoolap` se leyó en el código fuente fijado en
> `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/stoolap-0.4.0/`, no se recordó de las
> entradas. Donde la línea base contradice una entrada, queda registrado como **Corrección de
> Evidencia**, no se aplica en silencio.

## Enfoque técnico

Un crate nuevo hacia las hojas del grafo con cuatro dependencias, una tabla, un índice único, una
forma de transacción y un arnés de conformidad compartido que tres backends consumen sin que ninguno
lo posea.

El adaptador es una implementación **síncrona simple**: `Repository` es un trait síncrono
(`crates/persistence-api/src/persistence/repository.rs:21-39`) y toda llamada a `stoolap` es
síncrona, así que ni `PostgreSQLRepository::block_on`
(`crates/persistence/src/postgres/repository.rs:51-53`) ni `StoolapEffectStore::run_blocking`
(`crates/effect-store/src/stoolap/mod.rs:227-236`) tienen razón de existir aquí. Ambos puentes
resuelven un desajuste que este crate no tiene; D-4 ya lo dijo, y el árbol de módulos de abajo no
deja lugar donde reintroducir ninguno por reflejo.

Dos decisiones sostienen el cambio, y ambas se resuelven aquí y no durante la aplicación. **D-5**
reemplaza la división de tenant de PostgreSQL basada en dos índices parciales por una columna
centinela `NOT NULL` más un único índice sencillo, porque Stoolap omite por completo la verificación
de unicidad ante `NULL`. **D-6** fija `sync=full` de forma literal dentro de un único constructor de
DSN, porque el modo por defecto de Stoolap no hace fsync por commit y su parser de DSN falla *hacia
la permisividad* ante un valor de sincronización no reconocido.

**No se incluye diagrama de secuencia**, y es una decisión de aplicabilidad, no una omisión. La regla
de diseño de `openspec/config.yaml` pide uno para flujos asíncronos complejos; este adaptador no
tiene ningún flujo asíncrono y su ruta de llamada más larga son tres sentencias dentro de una
transacción, escrita completa en AD-5. Las estructuras que sostienen el peso aquí son el **esquema**
y la **tabla de mapeo de conflictos**, ambas dadas por completo.

---

## Correcciones de Evidencia

Siete. Cada una se encontró leyendo la línea base o el código fuente fijado, no las entradas, y cada
una cambia lo que la implementación debe hacer.

### EC-1 — `InMemoryRepository` y `PostgreSQLRepository` **ya están en desacuerdo**, y el arnés no puede cubrir el caso en que difieren

Este es el hallazgo de mayor consecuencia del cambio, y cae exactamente donde la propuesta previó que
podía caer (RK-5, NG-9, KD-3) — antes de que exista una sola línea de código Stoolap, que es
precisamente para lo que servía el orden del Enfoque.

Para un agregado **nuevo** (sin fila / sin entrada) guardado con un `expected_version` **distinto de
cero**:

| Implementación | Comportamiento | Evidencia |
|---|---|---|
| `InMemoryRepository` | `current` es `0`, `current != expected_version`, por tanto **`Conflict { expected, actual: 0 }`** | `crates/persistence-memory/src/persistence/repository.rs:40-48` |
| `PostgreSQLRepository` | `current_version` es `None`, por tanto `new_version = 1` — `expected_version` **nunca se inspecciona** y el guardado **tiene éxito** | `crates/persistence/src/postgres/repository.rs:100-101` |

`save(id, agg, tenant, 42)` sobre un agregado inexistente devuelve entonces `Ok(1)` en un adaptador
ya publicado y `Err(Conflict { expected: 42, actual: 0 })` en el otro. Ambos satisfacen la firma del
trait. Es el mismo tipo de defecto sobre el que se escribió
`crates/testkit/src/event_store.rs:1-20`, en la misma familia de puertos, encontrado del mismo modo.

La documentación del propio trait respalda la lectura en memoria: *"`expected_version`: verificación
de concurrencia optimista. Usar `0` para agregados nuevos."*
(`persistence-api/src/persistence/repository.rs:18`). Una verificación de concurrencia optimista que
se omite justo cuando la fila no existe no es una verificación.

**Consecuencias, en orden:**

1. **`StoolapRepository` implementa la semántica documentada** (AD-5): fila ausente +
   `expected_version` distinto de cero ⇒ `Conflict { expected, actual: 0 }`. Implementar a propósito
   el comportamiento de PostgreSQL, para que un arnés pase, sería codificar un defecto
   deliberadamente.
2. **El arnés no contiene este escenario** (AD-8). Incluirlo fallaría contra PostgreSQL, y NG-9/R11
   prohíben corregir un adaptador ya publicado dentro de este diff. Un arnés de conformidad afirma
   aquello en lo que las implementaciones están obligadas a coincidir; un caso en el que
   demostrablemente no coinciden es un reporte de defecto, no una prueba a colar.
3. **Queda registrado como deuda con un seguimiento nombrado** — KD-3 se vuelve concreto, y **F-5**
   es el cambio que las reconcilia (ver Seguimientos Nombrados). El arnés gana el escenario allí, no
   aquí.
4. **OQ-1** pide al usuario confirmar la dirección (memoria es canónica, PostgreSQL es el defecto)
   antes de que `sdd-tasks` escriba la lista de escenarios. Es **no bloqueante para el adaptador** y
   **bloqueante solo** para decidir si F-5 se abre contra PostgreSQL o contra la documentación del
   trait.

### EC-2 — `ego-testkit` ya depende de `ego-persistence-memory`, así que la ejecución del arnés contra Memory no requiere **ninguna** arista nueva

La fila de Áreas Afectadas de la propuesta da a `crates/persistence-memory/` una *"dependencia de
desarrollo + un objetivo de prueba que invoca el arnés compartido"*. Esa dependencia ya existe en la
otra dirección: `crates/testkit/Cargo.toml:20` —
`ego-persistence-memory = { path = "../persistence-memory" }`, dependencia **normal**.

Por tanto la ejecución de conformidad de Memory pertenece a `crates/testkit/tests/`, donde tanto el
arnés como `InMemoryRepository` ya están en alcance. `crates/persistence-memory/` **no se toca en
absoluto** — sin edición de `Cargo.toml`, sin objetivo de prueba, sin cambio de fuente — lo cual es
estrictamente mejor que el plan de la propuesta y elimina una arista de desarrollo
`foundation → tooling` que habría sido legal pero confusa (las aristas de desarrollo están excluidas
del grafo de capas, así que habría pasado el gate leyéndose como una violación). **AD-9.**

### EC-3 — El DDL de Stoolap acepta un `PRIMARY KEY` y silenciosamente no lo aplica

`crates/effect-store/src/stoolap/mod.rs:178-186` registra esto por experiencia directa contra esta
misma versión del crate: un `TEXT PRIMARY KEY` es **rechazado en tiempo de DDL**, un `PRIMARY KEY
(...)` compuesto a nivel de tabla se **parsea pero no se aplica en silencio** (sin restricción, sin
índice), y `UNIQUE (...)` **sí** se aplica plenamente, para una o varias columnas.

La identidad de la tabla equivalente a `aggregates` es `(tenant_id, aggregate_id)` — un compuesto de
dos columnas `TEXT`, que es la intersección de ambos modos de falla. Expresada como `PRIMARY KEY`
compilaría, se ejecutaría y no aplicaría nada; cada fila se duplicaría en cada guardado. **El esquema
expresa la identidad exclusivamente mediante `UNIQUE`, y la palabra `PRIMARY KEY` no aparece en este
crate.** AD-2.

### EC-4 — El registro de `Database` se indexa por la **cadena DSN completa**, así que el DSN debe tener una sola escritura

`DATABASE_REGISTRY` es un `FxHashMap<String, Arc<DatabaseInner>>` global al proceso indexado por el
DSN (`stoolap-0.4.0/src/api/database.rs:66-67`) y poblado con
`registry.insert(dsn.to_string(), …)` (`:324`). Dos llamadas a `Database::open` que nombran el mismo
directorio con cadenas de consulta distintas —por ejemplo `file:///data/db` y
`file:///data/db?sync=full`— son dos claves distintas, y por tanto **dos motores independientes sobre
un mismo directorio**.

No es hipotético: `file://{path}` es exactamente lo que construye el proveedor del effect-store
(`crates/effect-store/src/stoolap/mod.rs:175`), así que un segundo componente que abriera el mismo
directorio de la manera "obvia" obtendría su propio motor. La mitigación es estructural, no de
procedimiento: **una única función privada construye el DSN, recibe solo una ruta, y ningún sitio de
llamada del crate arma un DSN a mano** (AD-4). Cada handle para una ruta dada es entonces idéntico
byte a byte por construcción.

### EC-5 — Un valor `sync` no reconocido falla **hacia la permisividad**, en silencio

La rama de coincidencia de `parse_file_config` es
`"none"|"off"|"0" => None, "normal"|"1" => Normal, "full"|"2" => Full, _ => SyncMode::Normal`
(`stoolap-0.4.0/src/api/database.rs:430-436`). No hay rama de error. `sync=ful`, `sync=FULL ` con un
espacio de más, o `sync=true` resuelven todos a `SyncMode::Normal` —el valor por defecto sin fsync—
sin diagnóstico en ninguna parte.

Esto zanja de forma decisiva la sub-pregunta abierta de D-6: **un valor de sincronización
configurable o provisto por quien llama es peor que no tener perilla**, porque su modo de falla es
una degradación silenciosa de durabilidad que ningún error, log o tipo puede detectar. `sync=full` es
una constante literal (AD-4).

### EC-6 — `Database::dsn()` es público, lo que da a la decisión de durabilidad una superficie observable real

`pub fn dsn(&self) -> &str` (`stoolap-0.4.0/src/api/database.rs:1131-1133`). El requisito de R5 —*"el
modo de sincronización configurado se afirma en vez de asumirse"*— es entonces satisfacible mediante
una aserción contra el handle que el adaptador realmente abrió, no meramente contra una cadena que la
prueba reconstruye por su cuenta. La cláusula de pruebas de AD-4 depende de esto.

### EC-7 — El conflicto de reclamación de escritura MVCC **no tiene variante de error estructurada**; es `Internal` con un mensaje

`VersionStore::try_claim_row` devuelve
`Err(Error::internal(format!("row {} has uncommitted changes from transaction {}", …)))`
(`stoolap-0.4.0/src/storage/mvcc/version_store.rs:4453-4473`). `Error::Internal { message: String }`
(`core/error.rs:286`) es una variante de propósito general; las variantes dedicadas
`LockAcquisitionFailed(String)` (`:199`) y `DatabaseLocked` (`:236`) que `backend_err` ya clasifica
(`crates/effect-store/src/stoolap/mod.rs:91-98`) **no** se usan para este caso.

Por tanto el *"todo conflicto de escritura mapea a `Conflict`"* de D-7 no puede implementarse
únicamente con coincidencia de variantes: una rama debe coincidir con texto de mensaje. Eso es frágil,
y el diseño no finge lo contrario — se nombra, se acota a una sola rama, se hace **fallar de forma
ruidosa** (los errores no coincidentes siguen siendo `Internal`, nunca se vuelven `Conflict`), y se
fija con una prueba que corre dos transacciones reales en carrera, de modo que un cambio de mensaje
en un Stoolap futuro rompe la compilación en vez de reclasificar en silencio todo conflicto de
concurrencia como error interno. **AD-7.**

---

## Esquema

Una tabla, un índice único compuesto, cuatro columnas. Ejecutado por el constructor en cada apertura;
`IF NOT EXISTS` lo hace idempotente, exactamente como el DDL del proveedor del effect-store
(`crates/effect-store/src/stoolap/mod.rs:187-219`).

```sql
CREATE TABLE IF NOT EXISTS aggregates (
    tenant_id    TEXT    NOT NULL,
    aggregate_id TEXT    NOT NULL,
    version      INTEGER NOT NULL,
    payload      TEXT    NOT NULL,
    UNIQUE (tenant_id, aggregate_id)
)
```

| Columna | Tipo | Por qué |
|---|---|---|
| `tenant_id` | `TEXT NOT NULL` | El ámbito de tenant en su codificación centinela (AD-3). El `NOT NULL` es todo el punto: una columna anulable eludiría por completo la verificación de unicidad (D-5). Va primero para que el orden de columnas coincida con el del índice |
| `aggregate_id` | `TEXT NOT NULL` | El `&str` de identificador de quien llama, almacenado literalmente |
| `version` | `INTEGER NOT NULL` | El `INTEGER` de Stoolap tiene ancho `i64`, que coincide exactamente con el `i64` de versión de `Repository` — sin estrechamiento en ninguna parte |
| `payload` | `TEXT NOT NULL` | `serde_json::to_string(&aggregate)`. Stoolap no tiene tipo de columna JSON, y su `core::Value` tampoco tiene variante binaria (la razón por la que el proveedor del effect-store codifica bytes en base64, `mod.rs:11-14`); una **cadena** JSON no necesita ninguno de los dos rodeos |
| — | `UNIQUE (tenant_id, aggregate_id)` | El único índice que D-5 pide. En línea y no como `CREATE UNIQUE INDEX` separado: una sola sentencia, y es la forma exacta que el proveedor del effect-store ya demuestra que funciona contra stoolap 0.4.0 (`mod.rs:200,215`) |

**Sin `updated_at`.** La tabla `aggregates` de PostgreSQL lleva una
(`postgres/repository.rs:21`) porque su migración la define; nada en `Repository` la lee, este crate
no tiene puerto de retención ni de auditoría, y llevarla añadiría `chrono` al conjunto de
dependencias por una columna que ninguna ruta de código consume. Omitida deliberadamente, no por
descuido — si algún operador la necesita, es un cambio de esquema con una razón declarada. Es la
misma disciplina que NG-2 aplica a las abstracciones, aplicada a una columna.

**Sin `PRIMARY KEY`, en ninguna parte.** EC-3.

**Alternativa rechazada — una sentencia de índice separada.** `CREATE UNIQUE INDEX IF NOT EXISTS
idx_aggregates_scope ON aggregates (tenant_id, aggregate_id)` está soportado (el parser lee
`IF NOT EXISTS` para `CREATE INDEX` en `stoolap-0.4.0/src/parser/statements.rs:2084`) y sería
equivalente. Se rechaza solo porque es una segunda sentencia con un segundo modo de falla sin ganancia
alguna, y la forma en línea es la que tiene evidencia dentro del repositorio.

---

## Decisiones de Arquitectura

### AD-1 — Crate, capa y un conjunto de cuatro dependencias sin nada especulativo

**Decisión** — `crates/persistence-stoolap/`, paquete `ego-persistence-stoolap`, una línea nueva en
la tabla `[layers]` de `layers.toml` (hoy `:15-36`) y una en la lista `members` del workspace raíz
(`Cargo.toml:2-24`):

```toml
"ego-persistence-stoolap" = "infrastructure"
```

`xtask/src/layers.rs` **no se abre**.

```toml
[package]
name = "ego-persistence-stoolap"
version = "0.1.0"
edition = "2021"

[dependencies]
# El único puerto que implementa este crate, más `PersistenceError` y
# `resolve_tenant`. No `ego-domain`: nada aquí necesita un tipo de valor de
# dominio (propuesta D-3), a diferencia de ego-persistence-memory, que
# necesitaba `Clock`.
ego-persistence-api = { path = "../persistence-api" }
# Ya fijado en 0.4.0 en Cargo.lock vía la feature opcional `stoolap` de
# ego-effect-store. No entra ningún crate externo nuevo al workspace.
stoolap = "0.4"
# `Repository<A> for StoolapRepository<A, F>` acota `A: Serialize` — la misma
# cota que lleva PostgreSQLRepository (postgres/repository.rs:58).
serde = "1"
# `to_string` en la ruta de escritura; `serde_json::Value` es el tipo de
# entrada del closure deserializador, fijado por espejar la `F` de
# PostgreSQLRepository.
serde_json = "1"

[dev-dependencies]
# El arnés de conformidad compartido (AD-8). Una sola dirección: ego-testkit
# no depende de este crate, así que no hay ciclo. Misma forma que
# crates/effect-store/Cargo.toml:44.
ego-testkit = { path = "../testkit" }
# Cada prueba necesita su propio directorio de base de datos. Misma razón y
# misma versión que crates/effect-store/Cargo.toml:39.
tempfile = "3"
```

**Criterios**:

1. **`infrastructure` se confirma contra la matriz ejecutable, no se asume.** `layers.toml:10`
   permite `infrastructure → domain`, y `ego-persistence-api` es `domain` (`:17`), así que la única
   arista saliente que crea este crate ya es legal y **ninguna rama de la matriz cambia**. Los
   precedentes hermanos coinciden: `ego-persistence` (`:27`) y `ego-effect-store` (`:35`) son ambos
   `infrastructure`; `ego-persistence-memory` es `foundation` (`:36`) precisamente porque no maneja
   ningún backend, y este crate sí.
2. **La entrada es obligatoria, no opcional.** Las verificaciones de `xtask` cubren todo miembro del
   workspace cuyo manifiesto esté bajo `<raíz>/crates/`, así que un `crates/persistence-stoolap/` sin
   mapear falla la verificación de completitud de FR-001. Añadir la entrada *satisface*
   `foundation-integrity`; no lo modifica — confirmando la nota de Capacidades de la propuesta en vez
   de asumirla.
3. **Cuatro dependencias normales, cada una trazable a una línea de código**, y nada más:

   | Dependencia | Requerida por | Evidencia |
   |---|---|---|
   | `ego-persistence-api` | `Repository`, `PersistenceError`, `resolve_tenant` | `persistence-api/src/persistence/{repository.rs:12, error.rs:8, tenant.rs:29}` |
   | `stoolap` | `Database`, `Transaction`, `Error` | AD-4, AD-5, AD-7 |
   | `serde` | la cota `A: Serialize` del `impl` | AD-5 |
   | `serde_json` | `to_string` (ruta de escritura) y `Value` (parámetro del closure `F`) | AD-5, AD-6 |

4. **`async-trait` y `tokio` están ausentes a propósito** (D-4). Ningún método de trait aquí es
   `async`; los dos puentes asíncronos del repositorio
   (`postgres/repository.rs:51-53`, `effect-store/src/stoolap/mod.rs:227-236`) resuelven cada uno un
   desajuste que no existe en este crate. Su ausencia es una propiedad verificable del diff, no una
   promesa.
5. **`chrono` está ausente** porque el esquema no tiene columna de marca temporal (ver Esquema).
6. **R7 es un grep, no una afirmación.** Ningún token `sqlx`, `PgPool`, `ego-persistence`,
   `postgres` o de migración aparece en el manifiesto ni en ninguna parte bajo
   `crates/persistence-stoolap/`.

### AD-2 — Árbol de módulos: un tipo público, una ruta hacia él

```
crates/persistence-stoolap/               (paquete: ego-persistence-stoolap, capa infrastructure)
├── Cargo.toml                            # AD-1
├── src/
│   ├── lib.rs                            # doc de crate, `pub mod persistence;`, una reexportación raíz
│   └── persistence/
│       ├── mod.rs                        # `pub mod repository;`
│       └── repository.rs                 # StoolapRepository + SYSTEMWIDE_SCOPE + encode_tenant
│                                         #   + dsn_for + is_write_conflict + pruebas unitarias colocadas
└── tests/
    └── repository_conformance.rs         # la ejecución del arnés (AD-9)
```

`lib.rs` lleva `pub use persistence::repository::StoolapRepository;`. **Criterios**: la ruta de
módulo espeja exactamente `ego_persistence_api::persistence::repository`, como hace el árbol de
`ego-persistence-memory` (su diseño AD-3), *y* la raíz del crate reexporta su único tipo público,
como hace `ego-persistence` (`crates/persistence/src/lib.rs:11`). Los dos precedentes difieren porque
resolvían problemas distintos — persistence-memory tenía siete tipos en tres familias de puertos y
una matriz de compatibilidad que preservar; este crate tiene uno. Con un tipo, una ruta corta es todo
el argumento.

**Sin `#![deny(missing_docs)]`**, igual que ambos crates adaptadores hermanos
(`crates/persistence-memory/src/lib.rs:1`, `crates/persistence/src/lib.rs:1` — ninguno lleva atributo
de lint a nivel de crate). Los comentarios de documentación se escriben igual; esto solo declina
añadir un gate inconsistente con el workspace en un cambio cuyo tema no son los lints.

**Todo salvo `StoolapRepository` es privado.** `SYSTEMWIDE_SCOPE`, `encode_tenant`, `dsn_for` e
`is_write_conflict` son internos al crate, que es lo que hace que el argumento de no-fuga de AD-3 sea
estructural y no aspiracional.

### AD-3 — D-5 resuelto: el centinela de tenant, su frontera de una sola dirección, y por qué no puede colisionar

**Decisión** — tres elementos en `repository.rs`, y una regla:

```rust
/// La escritura en disco del ámbito systemwide (sin tenant).
///
/// Stoolap omite por completo la verificación de restricciones únicas cuando
/// cualquier columna indexada es NULL, y no tiene índices parciales — así que
/// la división en dos índices parciales de PostgreSQL
/// (postgres/repository.rs:114-148, migración 015) no tiene equivalente aquí,
/// y un `tenant_id` anulable dejaría acumular filas systemwide duplicadas para
/// un mismo agregado sin que nada emitiera error.
///
/// `""` es seguro como centinela porque `resolve_tenant` rechaza `Some("")`
/// como `MissingTenant` (persistence-api/src/persistence/tenant.rs:32) antes de
/// alcanzar cualquier adaptador, así que la cadena vacía nunca puede llegar
/// aquí como un tenant real.
const SYSTEMWIDE_SCOPE: &str = "";

/// Codifica un ámbito de tenant ya resuelto al valor que guarda la columna
/// `tenant_id`. El **único** lugar donde `Option<&str>` se vuelve valor de
/// columna.
fn encode_tenant(resolved: Option<&str>) -> &str {
    resolved.unwrap_or(SYSTEMWIDE_SCOPE)
}
```

**La regla — ninguna sentencia SQL de este crate selecciona jamás `tenant_id`.** Aparece solo en
predicados `WHERE` y en la lista de columnas del `INSERT`. No hay dirección de decodificación porque
no hay hacia dónde decodificar: ningún método de `Repository` devuelve un tenant
(`repository.rs:21-39` — `save` devuelve `i64`, `load` devuelve `A`, `delete` devuelve `()`).

**Criterios**:

1. **La frontera de codificación es exactamente una función, llamada en exactamente tres lugares** —
   las dos primeras líneas de `save`, `load` y `delete`, cada una con la misma forma:

   ```rust
   let resolved = resolve_tenant(tenant_id)?;          // MissingTenant escapa antes de todo SQL
   let scope = encode_tenant(resolved.as_deref());     // la única conversión Option -> columna
   ```

   Sin `unwrap_or("")` en línea, sin `.unwrap_or_default()` en un sitio de llamada, sin una segunda
   escritura del centinela. Este es el requisito de *"un único helper acotado, no lógica dispersa en
   línea"*, y es verificable por grep: `rg '""' crates/persistence-stoolap/src` debe devolver
   exactamente una línea fuera de pruebas, la declaración de `SYSTEMWIDE_SCOPE`.

2. **`decode_tenant` deliberadamente no se escribe.** Una inversa sin uso sería código especulativo
   que existe solo para sostener un argumento de simetría, y *debilitaría* la garantía de no-fuga al
   crear la única ruta de código que podría filtrar. La no-fuga es en cambio **estructural**: con
   `tenant_id` ausente de todo conjunto de resultados, no hay ruta desde la columna almacenada de
   vuelta a quien llama, así que R4 se cumple por construcción y no por revisión. Si algún puerto
   futuro necesita enumerar ámbitos, la dirección de decodificación se escribe entonces, junto con su
   consumidor.

3. **La prueba de no colisión, en cinco pasos** — cada paso es un hecho sobre una línea, no un juicio:

   1. Todo valor que llega a `tenant_id` es
      `encode_tenant(resolve_tenant(entrada_del_llamante)?.as_deref())` (criterio 1).
   2. `resolve_tenant` devuelve `Ok(None)` para `None`, `Err(MissingTenant)` para `Some("")`, y
      `Ok(Some(t))` para cualquier otro `Some(t)` (`tenant.rs:30-34`). Su rama `Some(t)` es por tanto
      alcanzable solo cuando `t != ""`.
   3. Entonces `encode_tenant` produce `""` **si y solo si** el ámbito resuelto era `None`, y una
      cadena no vacía en cualquier otro caso.
   4. El centinela y el conjunto de tenants reales almacenables son por tanto disjuntos, y
      `Option<String> → String` es inyectiva — dos ámbitos distintos nunca comparten un valor de
      `tenant_id`, y un mismo ámbito siempre produce el mismo.
   5. Inyectividad más `UNIQUE (tenant_id, aggregate_id)` da exactamente una fila por
      (ámbito, aggregate_id), **incluido el ámbito systemwide** — que es precisamente la garantía que
      una columna anulable habría perdido en silencio.

   El paso 2 es el que sostiene el peso, y ya está probado aguas arriba:
   `an_empty_tenant_is_rejected_rather_than_coerced_to_systemwide` (`tenant.rs:41-50`). Este diseño
   **consume** esa prueba en vez de duplicarla.

4. **Tres obligaciones de prueba, en tres niveles** (ninguna es reformulación de las otras):

   | Nivel | Prueba | Demuestra |
   |---|---|---|
   | Unitaria de crate | `encode_tenant_maps_only_the_absent_scope_to_the_sentinel` | El paso 3 directamente: `encode_tenant(None) == ""` y `encode_tenant(Some(t)) == t` para un `t` no vacío |
   | Unitaria de crate (SQL) | `two_systemwide_saves_leave_exactly_one_row` | **R3**, la falla que una columna anulable habría permitido. Guarda el mismo `aggregate_id` dos veces bajo `None`, luego afirma que `SELECT COUNT(*) FROM aggregates` es `1` y que `version` es `2`. Contar filas es interno al adaptador, así que esto no puede vivir en el arnés compartido |
   | Arnés compartido | los tres escenarios de tenant (AD-8) | **R4**: indistinguibilidad de comportamiento entre los tres backends, incluido `MissingTenant` ante `Some("")` |

5. **Alternativa rechazada — un centinela UUID aleatorio.** Eliminaría la colisión (ya imposible) al
   costo de una cadena mágica opaca en cada fila almacenada y un centinela sin relación demostrable
   con las reglas del propio puerto. `""` se elige *porque* `resolve_tenant` ya lo hace
   irrepresentable, lo cual es una prueba y no una improbabilidad.

### AD-4 — D-6 resuelto: `sync=full` es una constante literal en un único constructor de DSN

**Decisión** — una función privada, un constructor falible, sin perilla:

```rust
/// Construye el DSN que este adaptador abre para `path`.
///
/// `sync=full` es literal, y este es el único constructor de DSN del crate
/// (ver los criterios en design.md AD-4).
fn dsn_for(path: &Path) -> String {
    format!("file://{}?sync=full", path.display())
}

impl<A, F> StoolapRepository<A, F> {
    /// Abre (creándolo si no existe) un repositorio durable sobre Stoolap en `path`.
    pub fn new(path: &Path, deserialize: F) -> Result<Self, PersistenceError> {
        let db = Database::open(&dsn_for(path)).map_err(internal_err)?;
        db.execute(CREATE_AGGREGATES_TABLE, ()).map_err(internal_err)?;   // Esquema
        Ok(Self { db, deserialize, _marker: PhantomData })
    }
}
```

**Criterios**:

1. **Literal, no parámetro ni configuración** — la más fuerte de las tres opciones, y la razón es
   EC-5, no el gusto. El parser de DSN de Stoolap no tiene rama de error para un valor `sync` no
   reconocido: `_ => SyncMode::Normal` (`database.rs:435`). Una perilla cuyo error tipográfico
   devuelve en silencio el adaptador al modo por defecto sin fsync es peor que no tener perilla,
   porque la falla es invisible hasta que ocurre una caída y ningún error, log o tipo puede
   revelarla. Una constante no admite errores tipográficos del operador.
2. **Un parámetro también se consideró y se rechazó por motivos de cliente.** El punto 1 de la ronda
   de preguntas de la propuesta asume despliegues durables de un solo nodo, y D-6 declara que el
   estado de agregado no es una caché. No se ha identificado ningún consumidor que quiera el modo
   débil, y añadir el parámetro *antes* de que exista uno es el mismo movimiento especulativo que
   NG-2 rechaza para las abstracciones. Si aparece, es una propuesta con un cliente declarado —
   **F-6**.
3. **Un solo constructor de DSN cierra EC-4.** Como `dsn_for` recibe solo una ruta y ningún sitio de
   llamada arma un DSN, cada handle que este crate abre para un directorio dado es idéntico byte a
   byte, así que el registro indexado por DSN de Stoolap (`database.rs:66-67,324`) devuelve el
   *mismo* motor en lugar de un segundo sobre los mismos archivos. Una cadena de consulta literal y
   un único constructor son por tanto la misma decisión vista desde dos ángulos.
4. **El constructor es falible, y es una divergencia justificada respecto del *"espejar la forma
   pública de `PostgreSQLRepository`"* de IS-2.** `PostgreSQLRepository::new` es infalible
   (`postgres/repository.rs:43`) porque recibe un `PgPool` ya conectado y su esquema llega por una
   ejecución de migraciones que pertenece a otro. Este adaptador abre la base de datos *y* posee su
   esquema (IS-3), así que ambas cosas pueden fallar. `-> Result<Self, PersistenceError>` lo reporta
   en vez de entrar en pánico o diferir la falla al primer `save`. La forma genérica —`<A, F>`, el
   closure `deserialize`, `Debug` y las mismas cotas de trait— se preserva exactamente.
5. **La implementación de `Debug` imprime el DSN, no el handle.** La de `PostgreSQLRepository`
   imprime su pool (`:33-39`); la nuestra imprime `db.dsn()`, que es más útil y —por el criterio 1—
   no contiene ningún secreto, solo una ruta y un modo de sincronización fijo.

**Pruebas — qué es realmente verificable, dicho con honestidad:**

| Se entrega | Prueba | Qué demuestra |
|---|---|---|
| ✅ | `dsn_carries_full_sync` — función pura, `dsn_for(Path::new("/tmp/x"))` es igual a `"file:///tmp/x?sync=full"` | La cadena exacta, incluido el separador `?` y el literal `full` que acepta `database.rs:434` |
| ✅ | `an_opened_repository_requested_full_sync` — afirma que `repo.dsn()` (un accesor delgado sobre `Database::dsn()`, EC-6) es igual a `dsn_for(path)` | Que el constructor realmente usó `dsn_for` — cerrando la brecha entre "la función es correcta" y "la función se llama" |
| ✅ | `a_committed_save_survives_close_and_reopen` — guardar, soltar el repositorio, reabrir en la misma ruta, cargar | Respaldo en disco e idempotencia del DDL. **No fsync** |
| ❌ | una prueba real de recuperación ante caída / fsync | Ver abajo |

**La prueba de fsync no se escribe, y es un rechazo deliberado y no un hueco dejado abierto.** El
repositorio sí tiene pruebas reales de caída
(`integration-tests/tests/infrastructure/single_aggregate_crash_recovery_postgres.rs`, registrada en
`infrastructure.rs:117-122`), y matar un proceso hijo con SIGKILL es fácil de organizar. Pero un
*proceso* muerto no pierde nada que ya haya llegado a la caché de páginas del sistema operativo —
solo una *máquina* caída lo pierde. Tal prueba pasaría idénticamente con `sync=none`, `sync=normal` y
`sync=full`: **una prueba que no puede fallar por la razón que dice probar es peor que ninguna
prueba**, porque convierte una propiedad no verificada en una aparentemente verificada. Demostrar
fsync de verdad requiere un sistema de archivos con inyección de fallos o pérdida real de
alimentación, y ninguno de los dos pertenece a este cambio.

Así que las dos mitades de R5 se cumplen de formas distintas y el diseño dice cuál es cuál: *"un
guardado confirmado sobrevive un ciclo de cierre/reapertura"* lo demuestra la tercera prueba; *"el
modo de sincronización configurado se afirma en vez de asumirse"* lo demuestran las dos primeras. Que
el fsync efectivamente ocurra queda confiado a Stoolap, y esta línea es el registro de que esa
confianza es deliberada. **KD-4.**

### AD-5 — `save`: una transacción real, CAS sobre la versión, toda carrera perdida plegada a `Conflict`

**Decisión** — tres constantes de sentencia y un algoritmo. Todas las sentencias están
parametrizadas con marcadores `$n` y enlace por tupla, la forma que usa el proveedor del effect-store
en todo el archivo (`crates/effect-store/src/stoolap/mod.rs:416-417`); **ninguna cadena SQL de este
crate se construye interpolando un valor de quien llama**, así que no hay superficie de inyección
(Matriz de Amenazas).

```sql
-- SELECT_VERSION
SELECT version FROM aggregates WHERE tenant_id = $1 AND aggregate_id = $2

-- INSERT_AGGREGATE
INSERT INTO aggregates (tenant_id, aggregate_id, version, payload) VALUES ($1, $2, 1, $3)

-- UPDATE_AGGREGATE
UPDATE aggregates SET version = $1, payload = $2
 WHERE tenant_id = $3 AND aggregate_id = $4 AND version = $5
```

```
save(aggregate_id, aggregate, tenant_id, expected_version):

  1. resolved = resolve_tenant(tenant_id)?                     -> MissingTenant, antes de todo SQL
     scope    = encode_tenant(resolved.as_deref())             -> AD-3
     payload  = serde_json::to_string(&aggregate)?             -> Internal si falla

  2. tx = db.begin()?                                          -> Internal si falla, nunca Conflict
                                                                  (ReadCommitted; ver criterio 2)

  3. current: Option<i64> =
       tx.query(SELECT_VERSION, (scope, aggregate_id))?        -> primera fila, get::<i64>(0)
                                                                  Internal si falla

  4. new_version = según current:
       None                              -> 1                  si expected_version == 0
       None                              -> devolver Conflict { aggregate_id, expected: expected_version, actual: 0 }
       Some(c) si c == expected_version  -> expected_version + 1
       Some(c)                           -> devolver Conflict { aggregate_id, expected: expected_version, actual: c }
                                            (tx se descarta -> rollback automático de Stoolap)

  5. affected = según current:
       None    -> tx.execute(INSERT_AGGREGATE, (scope, aggregate_id, payload))
       Some(_) -> tx.execute(UPDATE_AGGREGATE, (new_version, payload, scope, aggregate_id, expected_version))
     si Err(e): is_write_conflict(&e) ? Conflict { expected: expected_version, actual: current.unwrap_or(0) }
                                      : Internal                                   -> AD-7

  6. si affected != 1:
       releer con SELECT_VERSION dentro de la tx aún abierta   -> `actual` veraz
       devolver Conflict { aggregate_id, expected: expected_version, actual: relectura.unwrap_or(0) }

  7. tx.commit()?  -> misma clasificación que el paso 5        -> AD-7
     Ok(new_version)
```

**Criterios**:

1. **La rama `None` del paso 4 implementa el contrato documentado, y es donde este adaptador
   deliberadamente no copia a PostgreSQL.** `postgres/repository.rs:100-101` devuelve `1` sin
   inspeccionar `expected_version`; `InMemoryRepository` entra en conflicto
   (`persistence-memory/src/persistence/repository.rs:40-48`); el trait documenta *"usar `0` para
   agregados nuevos"* (`persistence-api/src/persistence/repository.rs:18`). Este adaptador coincide
   con la documentación y con la lectura en memoria. **EC-1** contiene el hallazgo completo y sus
   consecuencias.
2. **`begin()` simple (ReadCommitted), no `begin_with_isolation(SnapshotIsolation)`.**
   `Database::begin` usa `ReadCommitted` por defecto (`stoolap-0.4.0/src/api/database.rs:995-997`), y
   el CAS `WHERE version = $5` provee el aislamiento que esta operación realmente necesita: cualquier
   intercalado que haya cambiado la fila invalida la guarda, así que un nivel más fuerte no aportaría
   nada y costaría rendimiento. El aislamiento de snapshot está disponible y se rechaza por ese
   motivo, no por descuido.
3. **`FOR UPDATE` no existe en Stoolap y no se emula.** La prevención de escritura sucia que
   `postgres/repository.rs:89-98` obtiene de un bloqueo de fila aquí proviene de la reclamación de
   escritura MVCC: `try_claim_row` rechaza la escritura de una segunda transacción sobre una fila que
   otra transacción retiene sin confirmar (`version_store.rs:4453-4473`). Mecanismo distinto, misma
   propiedad, y la clasificación del paso 5 es lo que la convierte en el mismo resultado *visible
   para quien llama*.
4. **El paso 6 existe por una carrera que el paso 4 no puede ver.** Bajo `ReadCommitted`, un par
   puede confirmar entre el SELECT y el UPDATE; la guarda `WHERE version = $5` no coincide con nada y
   `affected` es `0`. Releer dentro de la transacción abierta da un `actual` **veraz** en vez de uno
   plausible, lo cual importa porque la carga útil de `Conflict` es contra la que recarga quien
   reintenta. Si la fila desapareció por completo (un `delete` concurrente), `actual` es `0` — el
   mismo valor que reporta un agregado nuevo, que es exactamente lo que el almacén contiene ahora.
5. **`Conflict.actual` en el caso de reclamación de escritura es la última versión confirmada que
   observó esta transacción** (`current.unwrap_or(0)` del paso 5), no una conjetura: el escritor
   competidor no ha confirmado, así que aún no existe una versión posterior que reportar.
   `Conflict { expected: 5, actual: 5 }` es un resultado legítimo y honesto ahí, y la respuesta de
   quien llama —recargar y reintentar— es idéntica en ambos casos. Documentado en el método para que
   nadie lo lea como un error.
6. **`ON CONFLICT ... DO UPDATE` se consideró y se rechazó**, aunque la exploración confirmó que
   Stoolap lo soporta con coincidencia real de objetivo de conflicto. Con la columna centinela hay un
   solo índice, así que *funcionaría* — pero un upsert que sobrescribe no puede expresar *"solo si la
   versión sigue siendo `$expected`"*, que es todo el propósito de este método. Ramificar según el
   resultado del SELECT y guardar el UPDATE es más fuerte y más simple, y evita depender de una
   característica de dialecto en absoluto — una precaución que este repositorio ya pagó una vez
   (`DELETE ... WHERE col IN (SELECT ... LIMIT n)` borra cero filas en silencio en stoolap 0.4.0,
   `crates/effect-store/src/stoolap/mod.rs:242-251`).
7. **El rollback no necesita llamada explícita en las rutas de error.** `Transaction` hace rollback
   automático en `Drop` cuando no se ejecutó ni `commit` ni `rollback`
   (`stoolap-0.4.0/src/api/transaction.rs:493-530` y su `Drop`), así que cada `return Err(...)` de
   arriba ya es transaccional. Se dice para que nadie añada llamadas redundantes a `rollback()` que
   luego necesitarían su propio manejo de errores.

### AD-6 — `load` y `delete`: igualdad simple, porque la columna es `NOT NULL`

**Decisión**:

```sql
-- LOAD_PAYLOAD
SELECT payload FROM aggregates WHERE tenant_id = $1 AND aggregate_id = $2

-- DELETE_AGGREGATE
DELETE FROM aggregates WHERE tenant_id = $1 AND aggregate_id = $2
```

`load`: `resolve_tenant` → `encode_tenant` → consulta → sin fila ⇒ `NotFound { aggregate_id }`; con
fila ⇒ `serde_json::from_str::<serde_json::Value>(&payload)` y luego `(self.deserialize)(value)`.
`delete`: mismo prólogo → `execute` → `affected == 0` ⇒ `NotFound { aggregate_id }`, si no `Ok(())` —
coincidiendo exactamente con `postgres/repository.rs:205-209`.

**Criterios**:

1. **`=` es correcto aquí y `IS NOT DISTINCT FROM` sería ruido**, que es todo el beneficio de D-5.
   PostgreSQL necesita el operador seguro ante nulos porque su `tenant_id` es anulable y
   `NULL = NULL` nunca es `TRUE` (`postgres/repository.rs:82-88`; el incidente que registra
   `crates/testkit/src/event_store.rs:9-16`). Esta columna es `NOT NULL` y el ámbito systemwide es un
   valor ordinario, así que la lógica de tres valores nunca entra — un operador, una forma de
   predicado, ambos ámbitos.
2. **Ninguna de las dos sentencias selecciona `tenant_id`** — la regla de AD-3, y la mitad
   estructural de R4.
3. **`load` pasa por `serde_json::Value` en vez de ir directo a `A`.** El tipo del parámetro del
   closure `F` está fijado por espejar el de `PostgreSQLRepository` (`postgres/repository.rs:59`), que
   recibe un `Value` de la columna JSON de `sqlx`. Deserializar `TEXT` → `Value` → `F` mantiene un
   único deserializador utilizable sin cambios contra ambos backends, precondición para que el arnés
   compartido construya los tres de la misma manera (AD-8).
4. **`delete` corre fuera de una transacción, deliberadamente.** Es una sola sentencia; envolver una
   sentencia única en una transacción explícita añade dos modos de falla y no cambia ninguna
   semántica.

### AD-7 — Mapeo de errores: lista explícita de permitidos, una rama frágil nombrada, y un valor por defecto que falla ruidosamente

**Decisión** — un predicado privado, usado solo en la ruta de escritura (pasos 5 y 7 de `save`):

```rust
/// Si un error de Stoolap significa "perdiste una carrera de escritura" en
/// lugar de "el backend falló".
///
/// La rama `Internal` coincide con texto de mensaje, y esa fragilidad es
/// conocida, no un descuido: el conflicto de reclamación de escritura MVCC de
/// Stoolap no tiene variante dedicada — `try_claim_row` devuelve
/// `Error::internal("row {} has uncommitted changes from transaction {}")`
/// (version_store.rs:4453-4473). `race_between_two_transactions_is_a_conflict`
/// lo fija, así que un Stoolap futuro que cambie este mensaje rompe esa prueba
/// en vez de reclasificar en silencio todo conflicto de concurrencia como
/// error interno.
fn is_write_conflict(e: &stoolap::Error) -> bool {
    match e {
        stoolap::Error::UniqueConstraint { .. } => true,
        stoolap::Error::TransactionAborted => true,
        stoolap::Error::LockAcquisitionFailed(_) | stoolap::Error::DatabaseLocked => true,
        stoolap::Error::Internal { message } => {
            message.contains("uncommitted changes from transaction")
        }
        _ => false,
    }
}
```

| Error de Stoolap | → `PersistenceError` | Por qué |
|---|---|---|
| `UniqueConstraint { .. }` (`core/error.rs:104`) | `Conflict` | Dos transacciones vieron ambas que no había fila y ambas insertaron; el índice de D-5 rechazó la segunda. Una carrera perdida, exactamente |
| `Internal { message }` que coincide con el texto de reclamación (`version_store.rs:4463`) | `Conflict` | El equivalente MVCC de perder una carrera de `FOR UPDATE` — EC-7 |
| `TransactionAborted` (`:143`) | `Conflict` | La transacción perdió; recargar y reintentar es la respuesta correcta |
| `LockAcquisitionFailed(_)` (`:199`), `DatabaseLocked` (`:236`) | `Conflict` | Ver criterio 2 |
| **todo lo demás** | `Internal(e.to_string())` | Corrupción, desajuste de esquema, tabla ausente, falla de E/S — quien llama no debe reintentar |
| cualquier error fuera de la ruta de escritura de `save` (apertura, DDL, SELECT, serialización) | `Internal` | `is_write_conflict` nunca se consulta ahí. Una lectura que falla no es una carrera |

**Criterios**:

1. **La dirección por defecto es `Internal`, y esa elección sostiene peso.** Quien llama reintenta
   ante `Conflict`; mapear una falla no reconocida a `Conflict` metería a quien llama en un bucle de
   reintentos contra un backend permanentemente roto, y la falla nunca saldría a la superficie.
   Mapear una carrera genuina a `Internal` solo cuesta un error evitable. Equivocado en la dirección
   segura, a propósito.
2. **`LockAcquisitionFailed`/`DatabaseLocked` mapean a `Conflict`, y el intercambio se declara.**
   `backend_err` clasifica ambos como `EffectStoreError::TemporarilyUnavailable`
   (`crates/effect-store/src/stoolap/mod.rs:91-98`) — reintentable. `PersistenceError` no tiene
   variante reintentable, y `Conflict` es su única variante que quien llama ya reintenta, así que
   `Conflict` preserva el *comportamiento* correcto aunque la palabra sea imprecisa. La alternativa
   —`Internal`— convierte una contención transitoria en una falla dura. El riesgo residual es
   honesto: un bloqueo permanentemente atascado se presenta como un conflicto reintentable sin fin,
   acotado solo por la política de reintentos de quien llama.
3. **No se propone ninguna variante nueva de `PersistenceError`, y esto se verificó en vez de
   asumirse.** Cada modo de falla de Stoolap de arriba cae en una de las cuatro variantes existentes
   sin distorsión: `MissingTenant` lo emite aguas arriba `resolve_tenant`, `NotFound` cubre filas
   ausentes, `Conflict` cubre toda carrera perdida, `Internal` cubre fallas de backend. Lo único que
   las cuatro no pueden expresar es *"transitorio, reintenta"* como algo distinto de *"perdiste una
   carrera"* — y el criterio 2 muestra que la distinción **no tiene consecuencia de comportamiento
   para quien llama a un `Repository`**, ya que ambas respuestas son recargar y reintentar. Añadir
   una quinta variante reabriría un contrato ya publicado (NG-5, R6) para registrar una diferencia
   sobre la que nadie actúa. **Las cuatro variantes bastan; no se solicita ninguna adición de API.**
4. **La rama frágil se acota y se fija, no se esconde.** Es una rama, sobre una variante, verificada
   con `contains` sobre una subcadena estable frente a los parámetros del formato, y una prueba
   colocada corre dos transacciones reales en carrera sobre una fila para afirmar `Conflict` — así
   que la corrección de la rama se verifica contra el Stoolap fijado en vez de afirmarse sobre él.

### AD-8 — El arnés compartido: `assert_repository_conformance`, un agregado concreto, once escenarios

**Decisión** — `crates/testkit/src/repository_conformance.rs`, exportado desde
`crates/testkit/src/lib.rs` junto a sus tres hermanos (`:38`, `:41`, `:45-48`, `:51-54`):

```rust
/// Un agregado mínimo que posee el arnés, para que los tres backends se
/// juzguen contra la misma forma de carga útil y construyan el mismo
/// deserializador.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConformanceAggregate { pub value: String }

/// Construye un [`ConformanceAggregate`] que lleva `value`.
pub fn conformance_aggregate(value: &str) -> ConformanceAggregate { … }

/// Afirma que una implementación de [`Repository`] honra las partes del
/// contrato que tratan de *identidad y versionado* — qué filas pertenecen a
/// qué ámbito de tenant, cómo avanza una versión, y qué reporta una carrera
/// perdida.
///
/// # Pánico
///
/// Entra en pánico con un mensaje descriptivo en la primera divergencia del
/// contrato.
pub fn assert_repository_conformance<R>(repository: &mut R)
where
    R: Repository<ConformanceAggregate> + ?Sized,
{ … }
```

**Criterios**:

1. **`crates/testkit/src/event_store.rs` es la plantilla más cercana, no el `conformance.rs` de
   `ego-effect-store`** — D-9 ya decidió el *hogar*; esto fija la *forma*. Se hereda concretamente:
   el nombre `assert_*_conformance`, `&mut S` más `?Sized`, una instancia de almacén con un
   `aggregate_id` distinto por escenario (así el arnés no necesita forma de construir uno nuevo — lo
   cual importa porque la construcción es falible para Stoolap, infalible para Memory y dependiente
   del pool para PostgreSQL), pánico con mensaje ante divergencia, y un comentario de documentación
   que declara qué **no** se verifica deliberadamente.
2. **El tipo de agregado es concreto, donde el de `event_store.rs` es genérico.** Ese arnés es
   genérico sobre `E: DomainEvent` porque `DomainEvent` es un trait con comportamiento del que
   depende el contrato (se afirma `event_type()`). La `A` de `Repository<A>` no tiene trait ni
   comportamiento — toda propiedad bajo prueba (alcance de tenant, aritmética de versión, mapeo de
   errores) es completamente independiente de ella. Una `A` concreta quita dos cotas y un parámetro
   de closure, no quita nada de valor, y hace que los tres sitios de llamada sean casi idénticos.
   `integration-tests/.../repository_tenant_scoping_postgres.rs:37-53` ya llegó localmente a la misma
   conclusión con su propio `TestAggregate`; esto lo eleva.
3. **Que `ConformanceAggregate` pertenezca a `ego-testkit` es lo que hace que un deserializador sirva
   a dos backends.** `PostgreSQLRepository` y `StoolapRepository` necesitan ambos
   `F: Fn(serde_json::Value) -> Result<A, PersistenceError>`; con `A` fijada por el arnés, ambos
   sitios escriben el mismo closure de cuatro líneas (`:48-51` del archivo citado ya es exactamente
   ese closure). `ego-testkit` ya lleva `serde` con `derive` (`Cargo.toml:16`), así que no se añade
   ninguna dependencia.
4. **Once escenarios, cada uno con su propio `aggregate_id`.** Los primeros ocho son el núcleo del
   contrato; los últimos tres son las tres pruebas de `repository_tenant_scoping_postgres.rs`
   generalizadas a cualquier implementación, al nivel de rigor de ese archivo (IS-5).

   | # | Escenario | Afirma |
   |---|---|---|
   | 1 | un guardado nuevo empieza en versión 1 | `save(id, a, t, 0) == Ok(1)` |
   | 2 | guardados secuenciales avanzan la versión | `save(id, a, t, 1) == Ok(2)`, luego `Ok(3)` |
   | 3 | un `expected_version` obsoleto genera conflicto, con veracidad | `Conflict { expected, actual }` con **ambos campos de la carga útil verificados**, no solo la variante |
   | 4 | `load` hace ida y vuelta del agregado | el valor cargado es igual al guardado, y también al del *segundo* guardado tras una actualización |
   | 5 | cargar un agregado ausente es `NotFound` | variante y `aggregate_id` |
   | 6 | borrar un agregado ausente es `NotFound` | ídem |
   | 7 | `delete` elimina, y el `load` posterior es `NotFound` | el borrado es real, no una lápida |
   | 8 | `Some("")` es `MissingTenant` en **los tres métodos** | la mitad visible de R4; el centinela es invisible |
   | 9 | el ámbito systemwide hace ida y vuelta por save → load → save → delete | versión `1` y luego `2` (un ámbito invisible para su propia verificación de versión devolvería `1` dos veces), un conflicto por versión obsoleta bajo `None`, y `NotFound` tras el borrado |
   | 10 | dos tenants que comparten un `aggregate_id` no colisionan | filas independientes, cada una con su valor, y ninguna visible bajo `None` |
   | 11 | un ámbito de tenant y el systemwide no colisionan | filas independientes, y borrar la systemwide deja intacta la del tenant |

5. **Deliberadamente no cubierto, cada caso con su razón declarada** — un arnés que afirma más que el
   contrato convierte cada adaptador en una copia de aquel contra el que se escribió
   (`event_store.rs:47-51`):
   - **Un agregado nuevo con `expected_version` distinto de cero** — las dos implementaciones ya
     publicadas difieren (**EC-1**), y NG-9/R11 prohíben corregir una aquí. Registrado como
     **KD-3/F-5**, y el escenario se añade allí. El comentario de documentación del arnés lo dice por
     completo, así que la omisión es un hallazgo documentado y no un hueco invisible.
   - **Durabilidad** — no está en el contrato de `Repository`; `is_durable()` no existe en este
     trait. Se fija por adaptador (AD-4).
   - **Concurrencia** — un arnés compartido no puede construir un segundo handle sin conocer el
     backend. Se fija por adaptador (la prueba de carrera de AD-7).
   - **Forma de la carga útil** — los adaptadores son genéricos sobre `A`; afirmar un formato de
     serialización probaría `serde`, no el puerto.
6. **El orden de RK-4 es un prerrequisito duro, no un consejo.** El arnés se escribe y se pone en
   verde contra las dos implementaciones existentes *antes* de que exista código Stoolap — que es
   exactamente cómo se encontró EC-1, y es toda la razón por la que S1 es un corte separado (AD-11).

### AD-9 — Tres sitios de llamada, y uno de ellos no cuesta ninguna dependencia

**Decisión**:

| Backend | La ejecución vive en | Dependencia añadida | Por qué |
|---|---|---|---|
| `InMemoryRepository` | `crates/testkit/tests/repository_conformance_memory.rs` | **ninguna** | `ego-testkit` ya depende de `ego-persistence-memory` (`Cargo.toml:20`) — **EC-2**. `crates/persistence-memory/` no se toca en absoluto |
| `StoolapRepository` | `crates/persistence-stoolap/tests/repository_conformance.rs` | `ego-testkit` + `tempfile`, ambas de desarrollo (AD-1) | Misma forma que `crates/effect-store/Cargo.toml:44`. Una dirección; `ego-testkit` no depende de este crate, así que no hay ciclo |
| `PostgreSQLRepository` | `integration-tests/tests/infrastructure/repository_conformance_postgres.rs`, registrado como una línea `mod` en `integration-tests/tests/infrastructure.rs` | **ninguna** | `integration-tests` ya depende en desarrollo de `ego-testkit` (`Cargo.toml:59`) y de `ego-persistence` (`:54`) |

**Criterios**:

1. **D-10 se cumple estructuralmente.** La ejecución de PostgreSQL está en el workspace separado que
   posee el contenedor (`integration-tests/Cargo.toml:1-15`); la de Stoolap es embebida y respaldada
   en archivo, así que solo necesita un directorio de `tempfile` y se queda en la suite raíz.
   **Ninguna dependencia de Testcontainers o Docker entra al workspace raíz** (NG-8, R9) —
   `cargo test --workspace` sigue pasando sin runtime de contenedores disponible.
2. **Dos de los tres sitios de llamada no añaden nada a ningún manifiesto**, que es lo que hace que
   el *"declarado una vez, consumido por cada backend"* de R13 sea barato y no aspiracional.
3. **Cada sitio de llamada son unas cinco líneas**: construir el repositorio, llamar al arnés.
   Idénticos salvo la construcción — lo cual es en sí evidencia de que el arnés juzga el puerto y no
   una implementación.
4. **`repository_tenant_scoping_postgres.rs` se queda exactamente como está**
   (`infrastructure.rs:109-112`). Sus tres pruebas pasan a ser un subconjunto de lo que cubre el
   arnés, pero también documentan *por qué* PostgreSQL real es necesario para ellas (`:12-22` —
   ningún doble en memoria puede tergiversar `NULL = NULL`), y borrarlo borraría ese razonamiento. El
   solapamiento entre un arnés general y una prueba de regresión específica de backend es normal;
   NG-9 y R11 lo dejan intacto de todos modos.

### AD-10 — Las abstracciones que tentaron a este diseño, nombradas y rechazadas

La regla de la auditoría es *ninguna abstracción antes de dos consumidores claros*, y este cambio es
donde esa regla se ejercita en vez de citarse. Surgieron cuatro tentaciones al escribir este
documento. Cada una queda registrada con lo que habría abstraído y por qué se rechaza **ahora** — no
evitada en silencio.

| Tentación | Qué extraería | Rechazada porque |
|---|---|---|
| **Un helper compartido `TenantScope` de codificación/decodificación** en `ego-persistence-api`, ya que ahora tres adaptadores codifican un ámbito de tenant | `encode_tenant` + la constante centinela | Las tres codificaciones son *genuinamente distintas* — una clave de `HashMap` que lleva un `Option`, una columna SQL anulable leída con `IS NOT DISTINCT FROM`, y una columna centinela `NOT NULL`. Un helper compartido tendría que servir a las tres y colapsaría en `resolve_tenant`, que ya existe y ya es compartido. Lo común es la *regla*, y la regla ya está factorizada (`tenant.rs`) |
| **Un `SqlRepository<D: Dialect>`**, ya que los cuerpos de `save` de PostgreSQL y Stoolap ahora riman visiblemente | el texto de las sentencias + el algoritmo CAS tras un trait de dialecto | Riman y difieren donde importa: `FOR UPDATE` vs. reclamación de escritura MVCC, índices parciales vs. centinela, `IS NOT DISTINCT FROM` vs. `=`, asíncrono vs. síncrono, y —por EC-1— *semánticas de versión distintas*. Un trait de dialecto tendría que parametrizar las cinco, punto en el cual son dos implementaciones disfrazadas de tipo compartido. **NG-2, F-2** |
| **Una `StoolapConnection` compartida con `ego-effect-store`**, ya que ambos abren una base Stoolap y ambos clasifican sus errores | `Database::open` + DDL + clasificación de errores | Los dos quieren cosas **opuestas** de ambas: este crate fija `sync=full` mientras el effect store corre en el valor por defecto (KD-2), y `backend_err` mapea `LockAcquisitionFailed` a una variante *reintentable* que este puerto no tiene (AD-7 criterio 2). Compartir obligaría a uno de los dos a aceptar la postura de durabilidad y reintentos del otro. Dos consumidores, dos requisitos, una abstracción equivocada |
| **Generalizar el arnés a `assert_port_conformance<P>`**, ya que `ego-testkit` ahora tiene cuatro | las cuatro funciones `assert_*_conformance` tras un punto de entrada | Cuatro funciones con cuatro firmas sin relación y sin sitio de llamada común. La generalización es un nombre, no un mecanismo. **KD-1** ya registra que `Snapshot`, `OffsetStore` y `DedupStore` siguen sin ninguno; escribir esos es lo que produciría evidencia, y este no es ese cambio |

**Ninguna de las cuatro queda prohibida para siempre.** F-2 declara la condición: tres
implementaciones concretas de un *segundo* puerto, con la duplicación medida y no predicha. Un puerto
es un solo dato, y la propia regla de maduración arquitectónica de este repositorio exige dos o tres
recurrencias independientes antes de promover algo.

### AD-11 — Tres cortes, en el orden que fuerzan tanto las dependencias como RK-4

`sdd-tasks` posee la descomposición en tareas. Este diseño posee solo los límites, su orden, y la
razón por la que cada límite está donde está. RK-7 de la propuesta pronosticó una división en tres
cortes; esto lo confirma y aporta el argumento de la costura, porque el **arnés genuinamente puede
escribirse y ponerse en verde antes de que exista `StoolapRepository`** — eso no es una comodidad, es
lo que hace del arnés un juez en vez de un espejo del adaptador (RK-4).

| Corte | Contenido | Nuevas deps del crate | RED |
|---|---|---|---|
| **S1 — el arnés y sus dos sujetos existentes** | `crates/testkit/src/repository_conformance.rs` + un par `mod`/`pub use` en `lib.rs`; `crates/testkit/tests/repository_conformance_memory.rs`; `integration-tests/tests/infrastructure/repository_conformance_postgres.rs` + su línea `mod` | **ninguna** (EC-2, AD-9) | La prueba de Memory nombra `ego_testkit::assert_repository_conformance`, que aún no existe |
| **S2 — el crate, su esquema y sus sentencias** | `crates/persistence-stoolap/` (`Cargo.toml`, `lib.rs`, `persistence/{mod,repository}.rs`) con `new`/esquema/`load`/`delete` y el `save` completo; entrada en `layers.toml`; miembro del workspace; las pruebas unitarias colocadas (AD-3 criterio 4, AD-4, AD-7) | el conjunto de AD-1 | Una prueba local del crate nombra `StoolapRepository`, que aún no existe |
| **S3 — el tercer sujeto** | `crates/persistence-stoolap/tests/repository_conformance.rs`; las dependencias de desarrollo `ego-testkit` y `tempfile` | solo desarrollo | La ejecución del arnés no compila, y luego corre |

**Criterios**:

1. **S1 antes de S2 lo fuerza RK-4, no el tamaño.** Un arnés escrito después del adaptador es un
   arnés escrito según lo que el adaptador resulte hacer. La ejecución verde de S1 contra Memory y
   PostgreSQL es lo que lo calibra — y ya se pagó sola: **EC-1 es un hallazgo con forma de S1**,
   aflorado aquí en tiempo de diseño precisamente porque el orden se respetó en el análisis antes de
   respetarse en el código.
2. **S2 es una unidad revisable aunque sea la mayor.** Separar el esquema de `save` dejaría un crate
   cuya única prueba es que se puede crear una tabla — un corte que no demuestra nada y aun así
   cuesta una revisión. El esquema y el algoritmo CAS son una sola decisión (D-5 es *por qué* el CAS
   es expresable siquiera), y D-5 es la decisión que quien revisa más necesita ver entera.
3. **Se consideró y se rechazó una costura alternativa**: `new` + esquema + `load`/`delete` como un
   corte, y `save` como otro. Divide en un límite real —`save` es el único método con transacción—
   pero deja aterrizar el esquema centinela sin el código que hace que el índice único importe, así
   que R3 (la prueba de duplicado systemwide) no podría escribirse hasta el corte siguiente. **Si S2
   excede el presupuesto de revisión en la práctica, esta es la costura a usar**, con R3 moviéndose a
   la segunda mitad. Registrado para que el plan B sea una decisión y no una improvisación.
4. **Todo estado intermedio compila y todo corte previo sigue en verde.** Tras S1 el workspace tiene
   un arnés y dos sujetos que pasan, y ningún crate nuevo. Tras S2 tiene un crate no cableado del que
   nada depende. Tras S3, tres sujetos. La propiedad de rollback en pleno vuelo de la propuesta se
   sostiene en cada límite, y `xtask/src/layers.rs` nunca se abre, así que no hay estado de gate que
   deshacer.
5. **El TDD estricto es satisfacible en cada corte.** Cada RED es un fallo de compilación que nombra
   una ruta aún inexistente — la forma de RED que `ego-rs-testing-tdd` acepta.

---

## Puntos de Integración

| Frontera | Dirección | Mecanismo | Verificado en |
|---|---|---|---|
| `ego-persistence-stoolap` → `ego-persistence-api` | nueva, una dirección | dependencia `path`; `Repository`, `PersistenceError`, `resolve_tenant` | AD-1 |
| `ego-persistence-stoolap` → `stoolap` | nueva, una dirección | crates.io `0.4`, ya en `Cargo.lock` | AD-1 |
| `ego-persistence-stoolap` → cualquier otro crate del workspace | **ninguna** | no existe dependencia `path` | AD-1 criterio 6; R7 |
| `ego-persistence-stoolap` → `ego-testkit` | nueva, una dirección, **solo desarrollo** | consumo del arnés; excluida del grafo de capas | AD-9 |
| `ego-testkit` → `ego-persistence-memory` | **sin cambios** | ya es dependencia normal (`Cargo.toml:20`) | EC-2 |
| `integration-tests` → `ego-testkit`, `ego-persistence` | **sin cambios** | ya son dependencias de desarrollo (`:59`, `:54`) | AD-9 |
| `crates/persistence-memory/**` | **intacto** | sin cambio de manifiesto, fuente o prueba | EC-2, AD-9 |
| `crates/persistence-api/**` | **intacto** | ningún método, cota, supertrait, cuerpo por defecto o variante de error cambia | NG-5, R6; AD-7 criterio 3 |
| `crates/persistence/**` | **intacto** | sin SQL, migración, índice ni renombrado | NG-6, R7 |
| `crates/effect-store/**` | **intacto** | KD-2 se observa, no se corrige | KD-2 de la propuesta |
| `layers.toml` → `verify-layers` | entrada | una entrada nueva, cargador existente | AD-1 |
| `allowed_layers` → `check_direction` | **ninguna** | ninguna rama cambia; `infrastructure → domain` ya permitido (`layers.toml:10`) | AD-1 criterio 1 |
| Cableado de producción | **ninguno** | nada construye un `StoolapRepository`; un despliegue opta añadiendo la dependencia | Plan de Rollback de la propuesta |

**No se introduce ningún ciclo, y es un hecho sobre archivos y no una promesa de revisión.**
`ego-persistence-api` no nombra ninguna dependencia `path` del workspace, y `ego-persistence-stoolap`
solo la nombra a ella. La arista de desarrollo hacia `ego-testkit` entra a un sumidero `tooling`
(`layers.toml:34`) que no depende de este crate. Cargo rechazaría un ciclo antes de que corriera
`xtask verify-layers`, y la verificación de ciclos de FR-003 lo rechazaría otra vez — satisfaciendo
por construcción la regla *"Sin dependencias circulares entre crates"* de `openspec/config.yaml`.

## Estrategia de Pruebas

TDD estricto (`openspec/config.yaml` → `apply.tdd: true`). El RED de cada corte es un fallo de
compilación que nombra una ruta aún inexistente (AD-11).

| Nivel | Ubicación | Qué demuestra |
|---|---|---|
| Arnés compartido (principal) | `crates/testkit/src/repository_conformance.rs` | Los once escenarios de AD-8 — **R1, R2, R4, R13** |
| Ejecución del arnés — Memory | `crates/testkit/tests/repository_conformance_memory.rs` | **R2** sujeto 1; calibra el arnés contra una implementación conocida como buena (RK-4) |
| Ejecución del arnés — PostgreSQL | `integration-tests/tests/infrastructure/repository_conformance_postgres.rs` | **R2** sujeto 2, en el workspace que posee el contenedor (**D-10, R9**) |
| Ejecución del arnés — Stoolap | `crates/persistence-stoolap/tests/repository_conformance.rs` | **R1, R2** sujeto 3, en la suite raíz sin Docker |
| Unitaria de crate | `encode_tenant_maps_only_the_absent_scope_to_the_sentinel` | AD-3 paso 3 |
| Unitaria de crate (SQL) | `two_systemwide_saves_leave_exactly_one_row` | **R3** — una fila y versión `2`; la falla que una columna anulable habría permitido |
| Unitaria de crate | `dsn_carries_full_sync`, `an_opened_repository_requested_full_sync` | **R5** primera mitad (**D-6**, EC-5, EC-6) |
| Unitaria de crate | `a_committed_save_survives_close_and_reopen` | **R5** segunda mitad — respaldo en disco e idempotencia del DDL, explícitamente **no** fsync (AD-4, KD-4) |
| Unitaria de crate (concurrencia) | `race_between_two_transactions_is_a_conflict` | **R12** y la rama frágil de AD-7 — dos transacciones reales sobre una fila, afirmando `Conflict` y no `Internal` |
| Unitaria de crate | `a_stale_expected_version_is_a_conflict` | La otra mitad de **R12**, en el adaptador (el arnés también lo cubre; esta es la que falla primero cuando `save` regresiona) |
| Intacta | `crates/persistence-api/src/persistence/tenant.rs:41-50` | AD-3 paso 2, consumida y no duplicada |
| Intacta | `integration-tests/.../repository_tenant_scoping_postgres.rs` | **R11** — pasa sin modificación junto a la nueva ejecución del arnés (AD-9 criterio 4) |
| Gate | `cargo run -p xtask -- verify-layers` | **R8** — mapeado (FR-001), arista permitida (FR-002), sin ciclos (FR-003), compilación aislada (FR-005), **sin edición de matriz** |
| Workspace | `cargo test --workspace` sin runtime de contenedores | **R9** |
| Suite | `cargo run --manifest-path integration-tests/Cargo.toml --bin run-suite` | La ejecución de PostgreSQL |

Cinco propiedades son **propiedades del diff** — se verifican leyendo el cambio, no con una prueba:

- **R6** — `crates/persistence-api/**` no aparece en la lista de archivos.
- **R7** — `crates/persistence/**` no aparece en la lista de archivos, y ningún token `sqlx`,
  `PgPool`, `ego-persistence`, `postgres` o de migración aparece bajo `crates/persistence-stoolap/`.
- **R10** — el crate declara exactamente un `impl … for StoolapRepository`, y ningún `trait` propio.
- **AD-3 criterio 1** — `rg '""' crates/persistence-stoolap/src` devuelve exactamente una línea fuera
  de pruebas; `rg 'tenant_id' crates/persistence-stoolap/src` lo muestra solo en cláusulas `WHERE`,
  en la lista de columnas del `INSERT` y en el DDL — nunca en una lista `SELECT`.
- **AD-1 criterio 4** — ningún token `async`, `async_trait`, `block_in_place`, `spawn_blocking` o
  `tokio` aparece en el crate.

## Matriz de Amenazas

| Frontera | Exposición | Control |
|---|---|---|
| Construcción de SQL | Cinco constantes de sentencia, todas parametrizadas con `$n` y enlace por tupla | **Ninguna cadena SQL de este crate se construye interpolando un valor de quien llama.** `aggregate_id`, el ámbito de tenant, la carga útil y la versión son todos parámetros enlazados. Verificable por grep: ningún `format!` produce una sentencia en el crate |
| Aislamiento de tenant | El ámbito systemwide comparte una columna con todo tenant real | La prueba de inyectividad de AD-3 más `UNIQUE (tenant_id, aggregate_id)`. `resolve_tenant` falla de forma cerrada ante `Some("")` **antes de que corra SQL alguno**, así que un tenant vacío mal configurado nunca puede archivarse en la partición systemwide compartida (`tenant.rs:17-23`). Los escenarios 8–11 del arnés lo afirman por comportamiento en los tres backends |
| Fuga del centinela | Una codificación interna del adaptador podría llegar a quien llama | Estructural: ninguna sentencia selecciona `tenant_id`, y ningún método devuelve un tenant (AD-3 criterio 2). R4 |
| Datos en reposo | Las cargas útiles de agregado quedan en un archivo en texto plano en la ruta del operador | Postura sin cambios — la misma que el archivo Stoolap del proveedor del effect-store y que el directorio de datos de PostgreSQL. Este adaptador no añade cifrado y no lo declara; es el disco del despliegue el que hay que proteger |
| Credenciales | ninguna | El DSN lleva una ruta de sistema de archivos y un `sync=full` fijo. No existe contraseña, token ni endpoint de red que pueda filtrarse por `Debug` (AD-4 criterio 5) |
| Durabilidad | Una degradación silenciosa a commits sin fsync | AD-4: constante literal (el parser permisivo de EC-5 hace insegura una perilla), un único constructor de DSN (EC-4), y dos pruebas que lo fijan |

No hay frontera de enrutamiento, comando de shell, subproceso, automatización de VCS/PR,
clasificación de archivos ejecutables ni integración de procesos. Ninguna ruta de autenticación,
verificación de JWT o comprobación de `CrossTenantPermit` aparece en el diff.

## Migración / Despliegue

**Sin migración.** El crate crea su propia tabla en su propio archivo de base de datos, vía
`CREATE TABLE IF NOT EXISTS` en cada apertura (AD-4). Ningún dato, esquema o migración de un almacén
existente se lee, comparte o modifica en ninguna dirección. No se añade, edita ni referencia ningún
archivo de migración de PostgreSQL.

**Sin feature flag y sin despliegue por fases.** A diferencia de `ego-effect-store`, cuyos backends
son features opcionales (`crates/effect-store/Cargo.toml:46-50`) porque ese crate aloja *varios*
backends, este crate **es** el backend — un despliegue opta añadiendo la dependencia, y poner un
crate de un solo backend tras una feature de sí mismo es ceremonia sin consumidor.

El rollback es el de la propuesta, sin cambios, y está disponible en cada uno de los tres límites de
AD-11: eliminar `crates/persistence-stoolap/`, quitar el miembro del workspace y la entrada de
`layers.toml`, quitar el arnés de `ego-testkit` y sus tres sitios de llamada. Nada fuera del crate
nuevo depende de él; ningún archivo fuente existente cambia de comportamiento;
`xtask/src/layers.rs` nunca se abrió.

## Trazabilidad

| Elemento de propuesta / exploración | Resuelto por | Nota |
|---|---|---|
| D-1, IS-1 | AD-1, AD-2 | crate en `crates/persistence-stoolap/`, paquete `ego-persistence-stoolap` |
| D-2 | AD-1 criterios 1–2 | `infrastructure`, una línea en `layers.toml`, `xtask/src/layers.rs` intacto — confirmado contra la matriz, no asumido |
| D-3 | AD-1 criterio 3 | exactamente cuatro dependencias normales, cada una trazada a una línea; `ego-domain` genuinamente innecesaria |
| D-4 | AD-1 criterio 4, Enfoque técnico | sin puente asíncrono; la ausencia es una propiedad verificable del diff |
| **D-5**, IS-3, R3, R4, RK-1 | **AD-3**, Esquema, EC-3 | constante centinela + una función de codificación + la regla de no seleccionar `tenant_id`; prueba de no colisión en cinco pasos; DDL exacto; `PRIMARY KEY` rechazado |
| **D-6**, IS-4, R5, RK-2 | **AD-4**, EC-4, EC-5, EC-6, KD-4 | `sync=full` literal en un único constructor de DSN, con el parser permisivo como razón; qué se prueba y qué honestamente no |
| D-7, R12, RK-3 | **AD-7**, EC-7 | lista explícita de permitidos, una rama frágil nombrada y fijada por una prueba de carrera, defecto que falla ruidosamente; **no se requiere variante de error nueva** |
| D-8 | AD-5 | transacción real + escritura condicional guardada por versión; `FOR UPDATE` ausente y no emulado; `ON CONFLICT` considerado y rechazado |
| D-9, IS-5, R13 | **AD-8** | `assert_repository_conformance` en `ego-testkit`, la forma de `event_store.rs`, once escenarios, cuatro exclusiones declaradas |
| D-10, IS-6, R2, R9, NG-8 | **AD-9**, EC-2 | PostgreSQL solo en `integration-tests/`; Memory en las pruebas del propio `ego-testkit` a costo cero de dependencias; Stoolap en la suite raíz |
| D-11, NG-7, R7 | AD-1 criterio 6, propiedades del diff | verificable por grep, no afirmado |
| D-12, NG-1, NG-2, NG-3, R10, RK-6 | **AD-10** | cuatro abstracciones nombradas y rechazadas con razones, en vez de evitadas en silencio |
| NG-4, NG-6, F-3, F-4 | Puntos de Integración | `crates/runtime/`, `crates/effect-store/`, `crates/persistence/` ausentes de la lista de archivos |
| NG-5, R6 | AD-7 criterio 3 | las cuatro variantes existentes de `PersistenceError` bastan; verificado contra cada modo de falla de Stoolap, no asumido |
| **NG-9, R11, KD-3, RK-5** | **EC-1**, AD-5 criterio 1, AD-8 criterio 5, **OQ-1**, **F-5** | una divergencia real Memory/PostgreSQL encontrada en tiempo de diseño; Stoolap sigue el contrato documentado; el escenario se excluye del arnés y se archiva como deuda, no se corrige aquí |
| R14, F-1..F-4 | Seguimientos Nombrados | arrastrados, más F-5 y F-6 que abre este diseño |
| **RK-7** | **AD-11** | tres cortes confirmados con el argumento de la costura, más una costura de reserva nombrada si S2 aún excede el presupuesto |
| `config.yaml` "diagramas de secuencia" | Enfoque técnico | N/A explícito — sin flujo asíncrono; el esquema y la tabla de mapeo son las estructuras que sostienen el peso |
| `config.yaml` "sin dependencias circulares" | Puntos de Integración | una arista saliente hacia un crate sin dependencias propias del workspace |
| `config.yaml` "decisiones con justificación" | AD-1..AD-11 | cada una lleva criterios y, cuando existía, la alternativa rechazada |

## Deuda Conocida (añadida por este diseño)

- **KD-4** — `sync=full` se afirma en el DSN, y el fsync en sí queda confiado a Stoolap en vez de
  verificado. AD-4 explica por qué las formas de prueba disponibles no pueden fallar por la razón
  correcta. Registrado para que el límite de la garantía de R5 conste.

## Seguimientos Nombrados (añadidos por este diseño)

- **F-5** — **Reconciliar la semántica de `save` para agregados nuevos entre las tres
  implementaciones** (EC-1). `PostgreSQLRepository` ignora `expected_version` cuando no existe fila;
  `InMemoryRepository` y (por AD-5) `StoolapRepository` generan conflicto. Su propio cambio, con sus
  propias pruebas y su propia revisión de radio de impacto, por NG-9/R11. El duodécimo escenario del
  arnés le pertenece.
- **F-6** — Un modo de sincronización seleccionable, **solo** si aparece un despliegue que necesite
  el intercambio por rendimiento (AD-4 criterio 2). Deberá además resolver el parseo permisivo de
  EC-5, ya que una perilla que degrada en silencio ante un error tipográfico no es una forma
  aceptable.

## Preguntas Abiertas

- [ ] **OQ-1 — EC-1: ¿qué semántica de agregado nuevo es la canónica?** La documentación del trait e
      `InMemoryRepository` dicen que un `expected_version` distinto de cero sobre un agregado ausente
      es un conflicto; `PostgreSQLRepository` lo acepta en silencio. Este diseño implementa la lectura
      documentada y excluye el escenario del arnés (AD-8 criterio 5). **No bloqueante para el
      adaptador ni para los tres cortes.** Bloqueante solo para decidir cómo se abre F-5: contra
      `PostgreSQLRepository` como defecto, o contra la documentación del trait como lo equivocado.
- [ ] **OQ-2 — ¿Qué concurrencia promete la especificación?** El registro global al proceso de
      Stoolap (`database.rs:66-67`) significa que dos handles `StoolapRepository` sobre una misma
      ruta comparten un motor dentro del proceso, y su bloqueo de archivo gobierna el acceso entre
      procesos. El punto 2 de la ronda de preguntas de la propuesta asumió *"soportado, con
      características de concurrencia de un solo nodo declaradas con honestidad"*. `sdd-spec` necesita
      la afirmación concreta: un solo proceso y un solo nodo, o varios procesos y un solo nodo.
      **Bloqueante para `spec.md`, no para este diseño** — la forma de transacción de AD-5 es correcta
      en ambos casos.
- [ ] **OQ-3 — El constructor de AD-4 es falible**, divergiendo del *"espejar la forma pública de
      `PostgreSQLRepository`"* de IS-2 (`-> Self` allí, `-> Result<Self, PersistenceError>` aquí). La
      razón es que este adaptador abre la base de datos y posee su esquema, así que ambas cosas pueden
      fallar. Señalado para que la divergencia sea una decisión registrada y no una discrepancia que
      alguien descubra durante la revisión. **No bloqueante.**
