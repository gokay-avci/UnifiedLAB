# TODO: UnifiedLab Improvements

## 1. Draw.io Parser
- [ ] **Compressed XML Support**: Implement Base64 decoding and Flate2 inflation to read standard `.drawio` files.
- [ ] **Advanced Shapes**: Support `Switch` (Diamond), `Generator` (Hexagon), and `Aggregator` (Circle) nodes.
- [ ] **Parameter Parsing**: Extract parameters from node metadata/labels (e.g., `epsilon=0.5`).
- [ ] **Edge Types**: Differentiate between hard dependencies (Arrows) and soft dependencies (Data flow).

## 2. Core Engine
- [ ] **Agent Logic**: Implement more sophisticated decision strategies in `agent_shim.py`.
- [ ] **Physics Engines**: Add real VASP/GULP support (replace mocks).
- [ ] **Scalability**: Improve `NodeGuardian` to handle 1000+ jobs efficiently.

## 3. TUI & Monitoring
- [ ] **Interactive TUI**: Allow pausing/canceling jobs from the dashboard.
- [ ] **Resource Graphs**: Visualize CPU/GPU usage over time.
- [ ] **Log Filtering**: Better search/filter capabilities in the log view.

## 4. Testing & CI
- [ ] **Integration Tests**: End-to-end tests deploying real workflows.
- [ ] **Mock Improvements**: Better simulation of failures/latency in mock drivers.
- [ ] **Python Tests**: Add `pytest` suite for drivers.

## 5. Documentation
- [ ] **Tutorial**: Step-by-step video or guide creating a complex workflow.
- [ ] **API Reference**: Generate Rust docs (`cargo doc`).
