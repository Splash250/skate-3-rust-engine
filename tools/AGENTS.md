# Python tooling guidance

## Environment and checks

Run commands from the repository root so `tools.*` imports resolve. CI uses
Python 3.13. Use the task's Python environment (`python3` below; `python` on
Windows). Packaging dependencies are pinned in
[requirements-setup.txt](requirements-setup.txt); converter dependencies are in
[mixamo_to_skate/requirements.txt](mixamo_to_skate/requirements.txt).
If an import fails, check that environment and manifest before changing code.

Select the row matching the change; this is not a mandatory full-suite checklist.

| Change | Focused command |
| --- | --- |
| One pipeline module | `python3 -m unittest tools.asset_pipeline.test_<topic> -v` |
| Linux setup or shell launchers | `python3 -m unittest tools.test_setup_linux tools.test_setup_refresh tools.test_setup_assets tools.test_linux_scripts -v` |
| Installer publication, recovery or fingerprints | `python3 -m unittest tools.asset_pipeline.test_versions tools.asset_pipeline.test_setup_recovery -v` |
| Updater protocol | `python3 -m unittest discover -s tools -p 'test_update*.py' -v` |
| Character package update transaction | `python3 tools/mixamo_to_skate/check_package.py --transaction-only` |
| Animation converter | `python3 -m unittest discover -s tools/mixamo_to_skate -p 'test_*.py' -v` |

The converter's discovery directory matters: its tests import sibling modules
directly. For changes to `BUILD.sh` or `PLAY.sh`, also run
`sh -n BUILD.sh PLAY.sh`. Test commands use fixtures; real ISO extraction writes
installation data and is a separate integration step.

## Preserve installer behavior

- Preserve transactional publication, setup locking, receipt validation and the
  previous working installation on failure. Use temporary directories and
  synthetic data for regression tests.
- For exporter/input changes, inspect [asset_pipeline/versions.py](asset_pipeline/versions.py)
  and [asset_pipeline/customiser_setup.py](asset_pipeline/customiser_setup.py).
  Update the relevant source fingerprint so changed content refreshes without
  invalidating unrelated groups.
- Retain native extractor selection and the pinned Windows fallback in
  [asset_pipeline/install.py](asset_pipeline/install.py). See
  [docs/LINUX.md](../docs/LINUX.md) for actual owned-asset preparation.
- Owned-library tests are opt-in; read their environment-variable requirements.
  Missing NumPy/Pillow or private inputs are prerequisites to report, not reasons
  to weaken tests. Keep changes to `tools/vendor/` narrow and preserve its notices.
