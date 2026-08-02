# 01 — ApexAurum Nursery Analysis

## File Location & Role
`tools/nursery.py` — agent-callable tool surface for the "Nursery" subsystem inside ApexAurum (Claude-powered multi-agent Village ecosystem).

## High-Level Structure

```
NURSERY_DIR = sandbox/nursery/
├── datasets/          # *.jsonl
├── models/            # adapters + apprentice metadata
└── training_jobs.json # job history
```

Village event helper `_village_post_event` (tries vector_add_knowledge then village_post) with agent_id attribution (default NURSERY_KEEPER).

## Tool Groups

### Data Garden
- `nursery_generate_data(tool_name, num_examples=50, variation_level="medium", output_name=None, agent_id=None)`
  - Uses `training.synthetic_generator.SyntheticGenerator` (template-based, no LLM required for speed).
  - Falls back to live tool schemas from `ALL_TOOL_SCHEMAS`.
  - Writes JSONL; posts `dataset_created` event.
- `nursery_extract_conversations(source="sandbox/conversations.json", tools_filter=None, min_examples=10, ...)`
  - Mines real tool-use trajectories from chat history into training examples.
- `nursery_list_datasets(...)`

### Training Forge
- `nursery_estimate_cost(dataset_name, base_model="3b", epochs=3, provider="all")`
  - Calls `training.cloud_trainer.estimate_training_cost`; returns tokens / hours / $ per provider.
- `nursery_train_cloud(dataset_name, base_model, output_name, provider="together", epochs=3, lr=1e-5, lora_rank=16, agent_id=None)`
  - Primary path: Together / Replicate via `CloudTrainer`.
  - Vast.ai / RunPod currently return "use nursery_rent_gpu first" (incomplete in source).
  - Records job with trainer_agent; posts `training_started`.
- `nursery_train_local(dataset_name, base_model="TinyLlama/...", ..., use_cpu=False, agent_id=None)`
  - Synchronous `LoRATrainer` from `training.lora_trainer` (transformers + PEFT).
  - Suitable for 1–3B on consumer hardware; blocks the caller.
- `nursery_job_status(job_id)`, `nursery_list_jobs(status_filter=None, limit=20)`

### Model Cradle
- `nursery_list_models`, `nursery_deploy_ollama`, `nursery_test_model`, `nursery_compare_models`
  - Post-deployment registration and simple A/B.

### Apprentice Protocol (key for RSI)
- `nursery_create_apprentice(master_agent, apprentice_name, specialization, training_data_query=None, base_model=TinyLlama default, min_examples=20, auto_train=False)`
  - Queries Village knowledge for the master agent.
  - Converts knowledge → instruction data via `_convert_knowledge_to_training`.
  - Optionally kicks off local training.
  - Stores `{id}_apprentice.json` with lineage.
  - Posts events.
- `nursery_list_apprentices(master_filter=None, trained_only=False)`

## Schemas
All tools expose JSON Schema dicts (NURSERY_*_SCHEMA) for Claude tool-use registration. Standard pattern: name, description, input_schema with properties + required.

## External Dependencies (Python)
- `training.synthetic_generator`
- `training.cloud_trainer` (CloudProvider enum, CloudTrainer, estimate_training_cost)
- `training.lora_trainer` (LoRATrainer, TrainingConfig, check_dependencies)
- Village / vector_search modules (optional, graceful fallback)

## Gaps / Incomplete Bits in Source
- Vast.ai / RunPod training path is stubbed ("requires GPU rental workflow").
- Local training is blocking / synchronous.
- No explicit Together fine-tune API details (assumed inside CloudTrainer).
- Model registry is local JSON; no automatic push to HF Hub or Router.
- Apprentice base model defaults to TinyLlama (too small for serious specialist work with modern 27B drivers).

## Mapping to Modern Targets (2026)
- Base models: prefer Qwen3.6-27B, Qwen3.5-27B, Llama-3.x, etc.
- Providers: Together fine-tune API is mature and priced for 27B LoRA; Vast.ai for custom Unsloth containers.
- Storage / events: replace Village with CerebroCortex in ApexOS-RS.
- Deployment: prefer ApexRouter alias registration over pure Ollama (Router already supervises local + remote).
