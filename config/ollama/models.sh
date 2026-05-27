#!/usr/bin/env bash
# Ollama model management

OLLAMA_MODELS=("llama3.2" "qwen2.5" "mistral")

models_pull() {
  for model in "${OLLAMA_MODELS[@]}"; do
    echo "Pulling ${model}..."
    ollama pull "$model"
  done
}

models_list() {
  ollama list
}

models_health() {
  curl -s http://localhost:11434/api/tags | jq '.'
}
