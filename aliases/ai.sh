# AI environment aliases
alias models-up="docker compose -f config/litellm/docker-compose.yml up -d"
alias models-down="docker compose -f config/litellm/docker-compose.yml down"
alias models-health="curl -s http://localhost:11434/api/tags | jq '.'"
alias ollama-models="./config/ollama/models.sh"
