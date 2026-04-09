## v0.2.0 (2026-04-09)

### Feat

- **routes**: added patch config route to update config for each function
- **benchmark**: added benchmark binary for perf sampling
- added some info on response for testing purposes

### Fix

- removed prints to standart output in favor of logging
- **RandomBalancer**: fixed RandomBalancer bug when it continously picked first container in a group
- **container_manager.rs**: switched from docker exec + curl to direct http request into container

## v0.1.1 (2026-04-02)

### Fix

- **image-build**: image build log
