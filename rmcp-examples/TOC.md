The emergence of the Model Context Protocol is transforming how AI agents interact with tools, data, and real-world systems. However, most early MCP implementations rely on high-level runtimes that are not well-suited for embedded and resource-constrained edge environments. This session explores how RUST enables a new class of high-performance, memory-safe MCP servers designed specifically for Embedded Linux–powered edge devices.

In this tutorial, I'll walk through building a lightweight MCP server, bridging physical data sources into LLM-readable formats, enabling intelligent agents to reason over live edge data using Rust.

- Why MCP for Edge AI Systems?
- Why RUST?
- Building simple server using rmcp and testing with a client
- Bridging physical word e.g. Sensors, Telemetry, File Systems and structuring LLM-readable context, data pipelines
- High-performance Edge MCP Runtime - Async & Concurrency Models for scalable communication (MQTT, HTTP, gRPC etc.)
- Observability, tracing & Debugging
- Bring MCP in Agent loop, Using Rig for orchestration
- Deploying to target board, cross compilation steps
- Case Study: Building an Edge MCP Agent, e.g. Telemetry and Diagnostics
