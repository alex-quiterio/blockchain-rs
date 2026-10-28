#!/usr/bin/env bash
# A guided tour of a three-node network: people open wallets, miners mint coins, transfers
# gossip from node to node, blocks propagate, cheaters get turned away and two miners race.
# Every message is printed as it happens.
#
#   docker compose up --build        # in another terminal
#   ./scripts/story.sh
#
# Settings, from the environment:
#   NODES   space-separated node URLs   (http://127.0.0.1:3000 http://127.0.0.1:3001 http://127.0.0.1:3002)
#   ROUNDS  rounds of trading           (8)
#   PACE    seconds to pause per beat   (0.6)
#
# It uses the nodes' key and signing shortcuts (POST /wallets, /transactions/sign), which are for
# local experiments only. A real wallet signs where its key lives.

set -euo pipefail

read -r -a NODES <<< "${NODES:-http://127.0.0.1:3000 http://127.0.0.1:3001 http://127.0.0.1:3002}"
ROUNDS=${ROUNDS:-8}
PACE=${PACE:-0.6}
NODE_NAMES=(node-a node-b node-c)

for tool in curl jq perl; do
  command -v "$tool" > /dev/null || { echo "This needs $tool." >&2; exit 1; }
done

# ── Output ────────────────────────────────────────────────────────────────────

B=$'\e[1m' D=$'\e[2m' R=$'\e[0m'
RED=$'\e[31m' GRN=$'\e[32m' YEL=$'\e[33m' BLU=$'\e[34m' MAG=$'\e[35m' CYN=$'\e[36m'

now_ms() { perl -MTime::HiRes=time -e 'printf "%d", time*1000'; }
beat() { sleep "$PACE"; }
act() { printf '\n%s━━━ %s ━━━%s\n%s%s%s\n' "$B$MAG" "$1" "$R" "$D" "$2" "$R"; beat; }
node_label() { printf '%s%s%s' "$CYN" "${NODE_NAMES[$1]:-node-$1}" "$R"; }
short() { printf '%s…' "${1:0:8}"; }

# ── HTTP ──────────────────────────────────────────────────────────────────────

# Sets CODE and BODY.
call() {
  local method=$1 url=$2 data=${3:-}
  local out
  if [[ -n $data ]]; then
    out=$(curl -s -m 60 -w $'\n%{http_code}' -X "$method" -H 'content-type: application/json' -d "$data" "$url") || out=$'\n000'
  else
    out=$(curl -s -m 60 -w $'\n%{http_code}' -X "$method" "$url") || out=$'\n000'
  fi
  CODE=${out##*$'\n'}
  BODY=${out%$'\n'*}
}

status() { curl -s -m 5 "${NODES[$1]}/status"; }

# ── The cast ──────────────────────────────────────────────────────────────────

PEOPLE=(Alice Bob Carol Dave Erin Frank)
MINERS=(Mia Noah Olga)            # one per node
NAMES=() ADDR=() KEY=() NONCE=() HOME_NODE=()

who() {  # index of a name
  local i
  for i in "${!NAMES[@]}"; do [[ ${NAMES[$i]} == "$1" ]] && { echo "$i"; return; }; done
  return 1
}

open_wallet() {  # name home-node
  local node=$2
  call POST "${NODES[$node]}/wallets"
  NAMES+=("$1")
  ADDR+=("$(jq -r .address <<< "$BODY")")
  KEY+=("$(jq -r .secret_key <<< "$BODY")")
  NONCE+=(0)
  HOME_NODE+=("$node")
  printf '  👛 %-6s opens a wallet on %s → %s%s%s\n' "$1" "$(node_label "$node")" "$YEL" "$(short "${ADDR[${#ADDR[@]}-1]}")" "$R"
}

balance() { curl -s -m 5 "${NODES[0]}/balances/${ADDR[$1]}" | jq -r .balance; }

# ── Transactions ──────────────────────────────────────────────────────────────

LAST_TX=""

# Signs a transfer and sends it to the sender's home node. Returns non-zero if refused.
send() {  # from-index to-index amount fee
  local from=$1 to=$2 amount=$3 fee=$4 node=${HOME_NODE[$1]}
  local req
  req=$(jq -n --arg k "${KEY[$from]}" --arg to "${ADDR[$to]}" --argjson a "$amount" \
    --argjson n "${NONCE[$from]}" --argjson f "$fee" \
    '{secret_key: $k, to: $to, amount: $a, nonce: $n, fee: $f}')
  call POST "${NODES[$node]}/transactions/sign" "$req"
  LAST_TX=$BODY
  submit "$from" "$to" "$LAST_TX" "$amount" "$fee"
}

submit() {  # from-index to-index tx-json amount fee
  local from=$1 to=$2 tx=$3 amount=$4 fee=$5 node=${HOME_NODE[$1]}
  local fee_note=""
  (( fee > 0 )) && fee_note=" ${D}+fee $fee${R}"
  call POST "${NODES[$node]}/transactions" "$tx"
  if [[ $CODE == 202 ]]; then
    NONCE[$from]=$(( NONCE[$from] + 1 ))
    printf '  ✉️  %-6s ─%s %3d coins%s ─▶ %-6s via %s %s✓ %s accepted, gossiping%s\n' \
      "${NAMES[$from]}" "$GRN" "$amount" "$R$fee_note" "${NAMES[$to]}" "$(node_label "$node")" "$GRN" "$CODE" "$R"
  else
    printf '  ✉️  %-6s ─%s %3d coins%s ─▶ %-6s via %s %s✗ %s %s%s\n' \
      "${NAMES[$from]}" "$RED" "$amount" "$R$fee_note" "${NAMES[$to]}" "$(node_label "$node")" "$RED" "$CODE" \
      "$(jq -r '.message // .' <<< "$BODY" 2> /dev/null || echo "$BODY")" "$R"
    return 1
  fi
}

# Follows the last accepted transfer as gossip carries it into every node's mempool.
watch_gossip() {  # origin-node
  local origin=$1 sig started i
  sig=$(jq -r .signature <<< "$LAST_TX")
  started=$(now_ms)
  local -a seen=()
  for i in "${!NODES[@]}"; do seen+=(""); done
  while (( $(now_ms) - started < 5000 )); do
    local pending=0
    for i in "${!NODES[@]}"; do
      [[ -n ${seen[$i]} || $i == "$origin" ]] && continue
      if curl -s -m 5 "${NODES[$i]}/transactions/pending" | jq -e --arg s "$sig" 'any(.signature == $s)' > /dev/null; then
        seen[$i]=$(( $(now_ms) - started ))
      else
        pending=1
      fi
    done
    (( pending == 0 )) && break
    sleep 0.02
  done
  local line="     ${D}gossip${R}  "
  for i in "${!NODES[@]}"; do
    (( i == origin )) && continue
    if [[ -n ${seen[$i]} ]]; then
      line+="→ $(node_label "$i") ${GRN}+${seen[$i]}ms${R}   "
    else
      line+="→ $(node_label "$i") ${RED}not yet${R}   "
    fi
  done
  echo "$line"
}

mempools() {
  local i line="  📬 mempools  "
  for i in "${!NODES[@]}"; do
    line+="$(node_label "$i") $(status "$i" | jq -r .pending)   "
  done
  echo "$line"
}

# ── Blocks ────────────────────────────────────────────────────────────────────

describe_block() {  # block-json
  local block=$1
  jq -r --arg D "$D" --arg R "$R" '
    [.transactions[] | select(.signature != "")] as $txs
    | "  🧱 block #\(.index)  \($D)hash \(.hash[0:16])…  prev \(.prev_hash[0:16])…  nonce \(.nonce)\($R)",
      "     \($txs | length) transfer(s), \($txs | map(.fee // 0) | add // 0) coins in fees"' <<< "$block"
}

# Waits until every node holds the given height and prints when each one got there.
watch_propagation() {  # height started-ms origin-node
  local height=$1 started=$2 origin=$3 i h
  local -a seen=()
  for i in "${!NODES[@]}"; do seen+=(""); done
  local deadline=$(( started + 15000 ))
  while :; do
    local pending=0
    for i in "${!NODES[@]}"; do
      [[ -n ${seen[$i]} ]] && continue
      h=$(status "$i" | jq -r .height)
      if (( h >= height )); then
        seen[$i]=$(( $(now_ms) - started ))
      else
        pending=1
      fi
    done
    (( pending == 0 || $(now_ms) > deadline )) && break
    sleep 0.05
  done
  local line="  📡 propagation "
  for i in "${!NODES[@]}"; do
    if (( i == origin )); then
      line+="$(node_label "$i") ${D}mined it${R}   "
    elif [[ -n ${seen[$i]} ]]; then
      line+="$(node_label "$i") ${GRN}+${seen[$i]}ms${R}   "
    else
      line+="$(node_label "$i") ${RED}not yet${R}   "
    fi
  done
  echo "$line"
}

mine() {  # miner-index
  local miner=$1 node=${HOME_NODE[$1]}
  printf '  ⛏️  %s mines on %s…\n' "${NAMES[$miner]}" "$(node_label "$node")"
  local t0
  t0=$(now_ms)
  call POST "${NODES[$node]}/mine" "$(jq -n --arg m "${ADDR[$miner]}" '{miner: $m}')"
  if [[ $CODE != 201 ]]; then
    printf '  %s✗ %s %s%s\n' "$RED" "$CODE" "$BODY" "$R"
    return 1
  fi
  local found
  found=$(now_ms)
  printf '     found after %sms, the reward and fees go to %s\n' "$(( found - t0 ))" "${NAMES[$miner]}"
  describe_block "$BODY"
  watch_propagation "$(jq -r .index <<< "$BODY")" "$found" "$node"
}

difficulty() {
  local c
  c=$(curl -s -m 5 "${NODES[0]}/consensus")
  printf '  🎯 next block needs ~%s hashes %s(target pace %ss, next adjustment at height %s)%s\n' \
    "$(jq -r .work <<< "$c")" "$D" "$(jq -r .target_block_secs <<< "$c")" "$(jq -r .next_retarget <<< "$c")" "$R"
}

network() {
  local i s
  for i in "${!NODES[@]}"; do
    s=$(status "$i")
    printf '  %s %s  height %-3s tip %s  mempool %s\n' "$(node_label "$i")" "${D}${NODES[$i]}${R}" \
      "$(jq -r .height <<< "$s")" "$(jq -r '.tip[0:12]' <<< "$s")…" "$(jq -r .pending <<< "$s")"
  done
}

# ══ The story ═════════════════════════════════════════════════════════════════

act "Act 0 · The network" "Three nodes, each with its own copy of the chain, linked as peers."
for i in "${!NODES[@]}"; do
  status "$i" > /dev/null || { echo "${RED}${NODES[$i]} is not answering. Start the network first: docker compose up --build${R}"; exit 1; }
  peers=$(curl -s -m 5 "${NODES[$i]}/peers" | jq -r '[.[].url] | join(", ")')
  printf '  %s %s  peers: %s\n' "$(node_label "$i")" "${D}${NODES[$i]}${R}" "${peers:-none}"
done
network
difficulty

act "Act 1 · Wallets" "Everyone makes a keypair. The address is the public key; nothing touches the chain yet."
for i in "${!MINERS[@]}"; do open_wallet "${MINERS[$i]}" $(( i % ${#NODES[@]} )); done
for i in "${!PEOPLE[@]}"; do open_wallet "${PEOPLE[$i]}" $(( i % ${#NODES[@]} )); done

act "Act 2 · Coins from nothing" "Coins only enter through block rewards. Each miner mines on its own node; watch each block reach the others."
for m in "${MINERS[@]}"; do mine "$(who "$m")"; beat; done

act "Act 3 · Paying people" "Miners pay everyone. Each transfer is signed, sent to one node, and gossiped to the rest. It waits in every mempool until a block takes it."
for p in "${!PEOPLE[@]}"; do
  payer=$(who "${MINERS[$(( p % ${#MINERS[@]} ))]}")
  send "$payer" "$(who "${PEOPLE[$p]}")" $(( 10 + RANDOM % 10 )) 0 && watch_gossip "${HOME_NODE[$payer]}"
done
mempools
mine "$(who Noah)"
mempools

act "Act 4 · A busy market" "Rounds of transfers between people, through their own nodes, with fees for the miners. Miners take turns."
people_idx=()
for p in "${PEOPLE[@]}"; do people_idx+=("$(who "$p")"); done
for round in $(seq 1 "$ROUNDS"); do
  printf '\n  %sRound %s/%s%s\n' "$B" "$round" "$ROUNDS" "$R"
  for _ in $(seq 1 $(( 3 + RANDOM % 4 ))); do
    from=${people_idx[$(( RANDOM % ${#people_idx[@]} ))]}
    to=$from
    while [[ $to == "$from" ]]; do to=${people_idx[$(( RANDOM % ${#people_idx[@]} ))]}; done
    fee=0
    (( RANDOM % 2 )) && fee=$(( 1 + RANDOM % 3 ))
    send "$from" "$to" $(( 1 + RANDOM % 4 )) "$fee" || true
  done
  mempools
  mine "$(who "${MINERS[$(( round % ${#MINERS[@]} ))]}")"
  (( round % 4 == 0 )) && difficulty
  beat
done

act "Act 5 · Cheaters" "Every node checks every message on its own. Nobody has to trust the sender."
alice=$(who Alice) bob=$(who Bob) dave=$(who Dave)

echo "  ${B}Spending money you don't have${R}"
send "$dave" "$bob" 100000 0 || true
beat

echo "  ${B}Changing a signed transfer on the way (5 coins become 500)${R}"
call POST "${NODES[${HOME_NODE[$alice]}]}/transactions/sign" "$(jq -n --arg k "${KEY[$alice]}" --arg to "${ADDR[$bob]}" \
  --argjson n "${NONCE[$alice]}" '{secret_key: $k, to: $to, amount: 5, nonce: $n}')"
submit "$alice" "$bob" "$(jq '.amount = 500' <<< "$BODY")" 500 0 || true
beat

echo "  ${B}Replaying a transfer that already went through${R}"
send "$alice" "$bob" 2 0 || true
replay=$LAST_TX
mine "$(who Mia)"
NONCE[$alice]=$(( NONCE[$alice] - 1 ))  # pretend we don't know it was spent
submit "$alice" "$bob" "$replay" 2 0 || true
NONCE[$alice]=$(( NONCE[$alice] + 1 ))
beat

echo "  ${B}Skipping ahead with a nonce from the future${R}"
NONCE[$bob]=$(( NONCE[$bob] + 5 ))
send "$bob" "$alice" 1 0 || true
NONCE[$bob]=$(( NONCE[$bob] - 5 ))

act "Act 6 · A race" "Two miners on opposite ends of the network find a block at the same time. For a moment there are two truths; then the nodes agree on one."
send "$(who Carol)" "$(who Erin)" 3 1 || true
send "$(who Frank)" "$(who Alice)" 2 2 || true
sleep 0.5
last=$(( ${#NODES[@]} - 1 ))
race=$(mktemp -d)
t0=$(now_ms)
curl -s -m 60 -X POST -H 'content-type: application/json' -d "$(jq -n --arg m "${ADDR[$(who Mia)]}" '{miner: $m}')" "${NODES[0]}/mine" > "$race/Mia" &
curl -s -m 60 -X POST -H 'content-type: application/json' -d "$(jq -n --arg m "${ADDR[$(who Olga)]}" '{miner: $m}')" "${NODES[$last]}/mine" > "$race/Olga" &
wait
for racer in Mia Olga; do
  node=0
  [[ $racer == Olga ]] && node=$last
  if jq -e .hash "$race/$racer" > /dev/null 2>&1; then
    printf '  ⛏️  %-4s on %s found block #%s %s%s…%s\n' "$racer" "$(node_label "$node")" \
      "$(jq -r .index "$race/$racer")" "$YEL" "$(jq -r '.hash[0:12]' "$race/$racer")" "$R"
  else
    printf '  ⛏️  %-4s on %s %sgave up: %s%s\n' "$racer" "$(node_label "$node")" "$RED" \
      "$(jq -r '.message // .' "$race/$racer" 2> /dev/null || cat "$race/$racer")" "$R"
  fi
done
if [[ $(jq -r .index "$race/Mia" 2> /dev/null) == "$(jq -r .index "$race/Olga" 2> /dev/null)" ]]; then
  echo "  ${YEL}Same height, different blocks: the chain has forked.${R} Heaviest chain wins, ties go to the lower hash."
else
  echo "  ${D}No fork this time: one block arrived mid-search, so the other miner started over on top of it, as real miners do. Run it again for a split.${R}"
fi
rm -rf "$race"
for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
  tips=()
  for i in "${!NODES[@]}"; do tips+=("$(status "$i" | jq -r .tip)"); done
  printf '  🔀 +%5sms  ' "$(( $(now_ms) - t0 ))"
  for i in "${!NODES[@]}"; do printf '%s %s   ' "$(node_label "$i")" "${tips[$i]:0:10}…"; done
  if [[ $(printf '%s\n' "${tips[@]}" | sort -u | wc -l) -eq 1 ]]; then
    echo "${GRN}agreed${R}"
    break
  fi
  echo "${YEL}split${R}"
  sleep 0.5
done
winner=$(curl -s -m 5 "${NODES[0]}/chain?from=$(status 0 | jq -r .height)" | jq -r '.[-1].transactions[] | select(.signature == "") | .to')
for i in "${!ADDR[@]}"; do [[ ${ADDR[$i]} == "$winner" ]] && echo "  🏆 Every node now builds on ${NAMES[$i]}'s block. Transfers only in a losing block go back to the mempool."; done
mempools

act "Epilogue" "Where everyone stands, according to every node."
for i in "${!NAMES[@]}"; do
  printf '  %-6s %s%5s coins%s  %s\n' "${NAMES[$i]}" "$B" "$(balance "$i")" "$R" "${D}$(short "${ADDR[$i]}") sent ${NONCE[$i]} transfer(s)${R}"
done
echo
network
valid=$(curl -s -m 30 "${NODES[0]}/chain/valid" | jq -r .valid)
echo "  ✅ node-a replays its whole chain from genesis: valid = $valid"
