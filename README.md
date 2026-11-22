# FerrisCore

Base mínima de Ferris: **9 crates** sin verticales ni host wasm.

- `ferris-core`, `ferris-macro`, `ferris-db`, `ferris-schema`,
  `ferris-document`, `ferris-api`, `ferris-auth`, `ferris-web-shared`,
  `ferris-web-ui`.

El producto completo (Desk, verticales, WASM, servidor y CLI) vive en
[`FerrisFramework`](https://github.com/OpenFerris/FerrisFramework), que depende
de este repositorio por rutas `../../FerrisCore/ferris-*`. Los dos checkouts
deben estar **hermanos**:

```text
ferris/
├── FerrisCore/
└── FerrisFramework/
```

## Compilar

El entorno canónico es el contenedor de este repositorio (`Dockerfile` +
`compose.yaml`), con Rust 1.98.1 fijado:

```bash
docker compose run --rm ferris cargo check --workspace --all-targets
docker compose run --rm ferris cargo test --workspace -j 2 -- --test-threads=1
```

## Enlaces

- Framework y producto: <https://github.com/OpenFerris/FerrisFramework>
- Documentación: <https://openferris.github.io/FerrisDocs/>
- Reglas de desarrollo: [`AGENTS.md`](AGENTS.md)

## Ramas

`develop` es la integración (rama por defecto) y `master` es producción. Los
PR de trabajo van contra `develop`; `release/*` y `hotfix/*` son las únicas
ramas que se fusionan en `master`.
