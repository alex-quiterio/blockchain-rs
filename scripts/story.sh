#!/usr/bin/env bash
# A guided tour of a three-node network: people open wallets, miners mint coins, transfers
# gossip from node to node, blocks propagate, a full block takes the highest fees first,
# cheaters get turned away and two miners race. Every message is printed as it happens.
#
#   docker compose up --build        # in another terminal
#   ./scripts/story.sh
#
# Under proof of stake (CONSENSUS=pos docker compose up --build) the same cast forges instead of
# mining. Nobody stakes in the story, so anyone may forge every block.
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

for tool in curl jq perl shasum; do
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

# Sets CODE and BODY. Mining may take a while once the difficulty has risen.
call() {
  local method=$1 url=$2 data=${3:-}
  local out limit=10
  [[ $url == */mine || $url == */forge ]] && limit=300
  if [[ -n $data ]]; then
    out=$(curl -s -m "$limit" -w $'\n%{http_code}' -X "$method" -H 'content-type: application/json' -d "$data" "$url") || out=$'\n000'
  else
    out=$(curl -s -m "$limit" -w $'\n%{http_code}' -X "$method" "$url") || out=$'\n000'
  fi
  CODE=${out##*$'\n'}
  BODY=${out%$'\n'*}
}

status() { curl -s -m 5 "${NODES[$1]}/status"; }
consensus() { curl -s -m 5 "${NODES[0]}/consensus"; }

# Proof of work mines a block; proof of stake forges one, signed with the producer's key.
KIND=proof_of_work MINE=mine MINES=mines MINED=mined MINER=miner
produce_path() { [[ $KIND == proof_of_stake ]] && echo /forge || echo /mine; }
produce_body() {  # producer-index
  if [[ $KIND == proof_of_stake ]]; then
    jq -n --arg k "${KEY[$1]}" '{secret_key: $k}'
  else
    jq -n --arg m "${ADDR[$1]}" '{miner: $m}'
  fi
}

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
  printf '  👛 %-6s opens a wallet on %s → %s%s%s\n' "$1" "$(node_label "$node")" "$YEL" "${ADDR[${#ADDR[@]}-1]}" "$R"
}

balance() { curl -s -m 5 "${NODES[${2:-0}]}/balances/${ADDR[$1]}" | jq -r .balance; }  # who [node]

# ── Transactions ──────────────────────────────────────────────────────────────

LAST_TX=""
SENT=()  # "from to amount signature" of every accepted transfer

# Signs a transfer and sends it to the sender's home node. Returns non-zero if refused. With
# `quiet`, prints nothing.
send() {  # from-index to-index amount fee [quiet]
  local from=$1 to=$2 amount=$3 fee=$4 quiet=${5:-} node=${HOME_NODE[$1]}
  # Keys and addresses are hex and the rest are numbers, so printf builds the JSON without
  # starting jq for every transfer.
  call POST "${NODES[$node]}/transactions/sign" \
    "$(printf '{"secret_key":"%s","to":"%s","amount":%d,"nonce":%d,"fee":%d}' \
      "${KEY[$from]}" "${ADDR[$to]}" "$amount" "${NONCE[$from]}" "$fee")"
  LAST_TX=$BODY
  submit "$from" "$to" "$LAST_TX" "$amount" "$fee" "$quiet"
}

submit() {  # from-index to-index tx-json amount fee [quiet]
  local from=$1 to=$2 tx=$3 amount=$4 fee=$5 quiet=${6:-} node=${HOME_NODE[$1]}
  local fee_note=""
  (( fee > 0 )) && fee_note=" ${D}+fee $fee${R}"
  call POST "${NODES[$node]}/transactions" "$tx"
  if [[ $CODE == 202 ]]; then
    NONCE[$from]=$(( NONCE[$from] + 1 ))
    [[ $tx =~ \"signature\":[[:space:]]*\"([0-9a-f]+)\" ]]
    SENT+=("$from $to $amount ${BASH_REMATCH[1]}")
    [[ -n $quiet ]] && return 0
    printf '  ✉️  %-6s ─%s %3d coins%s ─▶ %-6s via %s %s✓ %s accepted, gossiping%s\n' \
      "${NAMES[$from]}" "$GRN" "$amount" "$R$fee_note" "${NAMES[$to]}" "$(node_label "$node")" "$GRN" "$CODE" "$R"
  else
    [[ -n $quiet ]] && return 1
    printf '  ✉️  %-6s ─%s %3d coins%s ─▶ %-6s via %s %s✗ %s %s%s\n' \
      "${NAMES[$from]}" "$RED" "$amount" "$R$fee_note" "${NAMES[$to]}" "$(node_label "$node")" "$RED" "$CODE" \
      "$(jq -r '.message // .' <<< "$BODY" 2> /dev/null || echo "$BODY")" "$R"
    return 1
  fi
}

# Sends `count` free transfers of a coin from one sender to `recipients` in turn, in nonce
# order, printing nothing until it's done, then how many were accepted. It changes no state of
# the story's, so several can run at once, one per sender: only each sender's own nonces need
# to arrive in order. Stops at the first refusal, since every later nonce would be refused too.
queue_free() {  # from-index count recipient-index...
  local from=$1 count=$2 node=${HOME_NODE[$1]} nonce=${NONCE[$1]} accepted=0 k tx
  shift 2
  local -a to=("$@")
  for (( k = 0; k < count; k++ )); do
    tx=$(curl -s -m 10 -H 'content-type: application/json' "${NODES[$node]}/transactions/sign" \
      -d "$(printf '{"secret_key":"%s","to":"%s","amount":1,"nonce":%d}' "${KEY[$from]}" "${ADDR[${to[$(( k % ${#to[@]} ))]}]}" "$nonce")")
    [[ $(curl -s -m 10 -o /dev/null -w '%{http_code}' -H 'content-type: application/json' \
      -d "$tx" "${NODES[$node]}/transactions") == 202 ]] || break
    accepted=$(( accepted + 1 )) nonce=$(( nonce + 1 ))
  done
  echo "$accepted"
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
    | (if .validator then "signed by \(.validator[0:8])…" else "nonce \(.nonce)" end) as $seal
    | "  🧱 block #\(.index)  \($D)hash \(.hash[0:16])…  prev \(.prev_hash[0:16])…  \($seal)\($R)",
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
      line+="$(node_label "$i") ${D}${MINED} it${R}   "
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
  printf '  ⛏️  %s %s on %s…\n' "${NAMES[$miner]}" "$MINES" "$(node_label "$node")"
  local t0
  t0=$(now_ms)
  call POST "${NODES[$node]}$(produce_path)" "$(produce_body "$miner")"
  # Another miner on the network can win the race; the story carries on either way.
  if [[ $CODE != 201 ]]; then
    printf '     %s✗ %s after %sms: %s%s\n' "$YEL" "$CODE" "$(( $(now_ms) - t0 ))" \
      "$(jq -r '.message // .' <<< "$BODY" 2> /dev/null || echo "${BODY:-no answer}")" "$R"
    return 0
  fi
  local found
  found=$(now_ms)
  printf '     %s after %sms, the reward and fees go to %s\n' "$MINED" "$(( found - t0 ))" "${NAMES[$miner]}"
  describe_block "$BODY"
  watch_propagation "$(jq -r .index <<< "$BODY")" "$found" "$node"
}

difficulty() {
  local c
  c=$(consensus)
  if [[ $KIND == proof_of_stake ]]; then
    printf '  🪙 proof of stake: %s\n' "$(jq -r 'if .next_validator then "the next block is \(.next_validator[0:8])…'"'"'s to forge" else "nothing is staked, so anyone may forge" end' <<< "$c")"
  elif [[ $(jq -r .target_block_secs <<< "$c") == null ]]; then
    printf '  🎯 every block needs ~%s hashes %s(fixed difficulty)%s\n' "$(jq -r .work <<< "$c")" "$D" "$R"
  else
    printf '  🎯 next block needs ~%s hashes %s(target pace %ss, next adjustment at height %s)%s\n' \
      "$(jq -r .work <<< "$c")" "$D" "$(jq -r .target_block_secs <<< "$c")" "$(jq -r .next_retarget <<< "$c")" "$R"
  fi
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
KIND=$(consensus | jq -r .kind)
if [[ $KIND == proof_of_stake ]]; then
  MINE=forge MINES=forges MINED=forged MINER=forger
fi
network
difficulty

act "Act 1 · Wallets" "Everyone makes a keypair. The address is the public key; nothing touches the chain yet."
for i in "${!MINERS[@]}"; do open_wallet "${MINERS[$i]}" $(( i % ${#NODES[@]} )); done
for i in "${!PEOPLE[@]}"; do open_wallet "${PEOPLE[$i]}" $(( i % ${#NODES[@]} )); done

act "Act 2 · Coins from nothing" "Coins only enter through block rewards. Each ${MINER} ${MINES} on its own node; watch each block reach the others."
for m in "${MINERS[@]}"; do mine "$(who "$m")"; beat; done

act "Act 3 · Paying people" "Miners pay everyone. Each transfer is signed, sent to one node, and gossiped to the rest. It waits in every mempool until a block takes it."
for p in "${!PEOPLE[@]}"; do
  payer=$(who "${MINERS[$(( p % ${#MINERS[@]} ))]}")
  send "$payer" "$(who "${PEOPLE[$p]}")" $(( 10 + RANDOM % 10 )) 0 && watch_gossip "${HOME_NODE[$payer]}"
done
mempools
mine "$(who Noah)"
mempools

act "Act 4 · A busy market" "Rounds of transfers between people, through their own nodes, with fees for whoever adds the block. The ${MINER}s take turns."
people_idx=()
for p in "${PEOPLE[@]}"; do people_idx+=("$(who "$p")"); done
for (( round = 1; round <= ROUNDS; round++ )); do
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

act "Act 5 · A full block" "A block holds only so many transfers. When more are waiting, whoever adds it takes the highest fees first, and the rest wait for the next one."
cap=$(consensus | jq -r '.max_block_transfers // empty')
if [[ -z $cap ]]; then
  echo "  ${D}This network sets no limit (MAX_BLOCK_TRANSFERS=0), so every block takes everything waiting.${R}"
else
  echo "  📏 at most ${B}${cap}${R} transfers a block"
  # More free transfers than fit, a coin each, from the ${MINER}s. If they can't afford that
  # many between them yet, one of them adds a block first.
  need=$(( cap + 6 ))
  left=() funds=0
  for m in "${MINERS[@]}"; do left+=("$(balance "$(who "$m")")"); funds=$(( funds + left[${#left[@]} - 1] )); done
  if (( funds < need )); then
    echo "  ${D}The ${MINER}s hold ${funds} coins between them, short of ${need}, so Mia adds blocks first.${R}"
    while (( funds < need )); do
      mine "$(who Mia)"
      funds=0 left=()
      for m in "${MINERS[@]}"; do left+=("$(balance "$(who "$m")")"); funds=$(( funds + left[${#left[@]} - 1] )); done
    done
  fi
  # Shares them out round-robin among those with coins left, then sends each share at once.
  share=()
  for m in "${MINERS[@]}"; do share+=(0); done
  for (( k = 0, given = 0; given < need && k < 3 * need; k++ )); do
    m=$(( k % ${#MINERS[@]} ))
    (( share[m] < left[m] )) && { share[m]=$(( share[m] + 1 )); given=$(( given + 1 )); }
  done
  queued=$(mktemp -d)
  for m in "${!MINERS[@]}"; do
    queue_free "$(who "${MINERS[$m]}")" "${share[$m]}" "${people_idx[@]}" > "$queued/$m" &
  done
  wait
  free=0
  for m in "${!MINERS[@]}"; do
    accepted=$(< "$queued/$m")
    free=$(( free + accepted ))
    NONCE[$(who "${MINERS[$m]}")]=$(( NONCE[$(who "${MINERS[$m]}")] + accepted ))
  done
  rm -rf "$queued"
  printf '  ✉️  the %ss queue %s%s free transfers%s of 1 coin\n' "$MINER" "$B" "$free" "$R"
  # ...then, last in line, a few people pay to jump the queue.
  paid=()
  for p in Alice Bob Carol Dave; do
    send "$(who "$p")" "$(who Erin)" 1 $(( 2 + RANDOM % 3 )) && paid+=("$(jq -r .signature <<< "$LAST_TX")")
  done
  mempools
  mine "$(who Noah)"
  if [[ $CODE == 201 ]]; then
    block=$BODY
    in_block=0
    for sig in "${paid[@]}"; do
      jq -e --arg s "$sig" 'any(.transactions[]; .signature == $s)' <<< "$block" > /dev/null && in_block=$(( in_block + 1 ))
    done
    waiting=$(status "${HOME_NODE[$(who Noah)]}" | jq -r .pending)
    if (( waiting == 0 )); then
      printf '  %sEverything fit this time, so nobody had to wait.%s\n' "$D" "$R"
    elif (( in_block == ${#paid[@]} )); then
      printf '  %s✓ all %s that paid a fee went in,%s though they arrived last. %s free transfer(s) wait for the next block.\n' \
        "$GRN" "$in_block" "$R" "$waiting"
    else
      printf '  %s✗ only %s of the %s that paid a fee went in, with %s left waiting.%s\n' \
        "$RED" "$in_block" "${#paid[@]}" "$waiting" "$R"
    fi
    mine "$(who Olga)"
    mempools
  fi
fi

act "Act 6 · Cheaters" "Every node checks every message on its own. Nobody has to trust the sender."
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

act "Act 7 · A race" "Two ${MINER}s on opposite ends of the network add a block at the same time. For a moment there are two truths; then the nodes agree on one."
send "$(who Carol)" "$(who Erin)" 3 1 || true
send "$(who Frank)" "$(who Alice)" 2 2 || true
sleep 0.5
last=$(( ${#NODES[@]} - 1 ))
race=$(mktemp -d)
t0=$(now_ms)
curl -s -m 300 -X POST -H 'content-type: application/json' -d "$(produce_body "$(who Mia)")" "${NODES[0]}$(produce_path)" > "$race/Mia" &
curl -s -m 300 -X POST -H 'content-type: application/json' -d "$(produce_body "$(who Olga)")" "${NODES[$last]}$(produce_path)" > "$race/Olga" &
wait
for racer in Mia Olga; do
  node=0
  [[ $racer == Olga ]] && node=$last
  if jq -e .hash "$race/$racer" > /dev/null 2>&1; then
    printf '  ⛏️  %-4s on %s %s block #%s %s%s…%s\n' "$racer" "$(node_label "$node")" "$MINED" \
      "$(jq -r .index "$race/$racer")" "$YEL" "$(jq -r '.hash[0:12]' "$race/$racer")" "$R"
  else
    printf '  ⛏️  %-4s on %s %sgave up: %s%s\n' "$racer" "$(node_label "$node")" "$RED" \
      "$(jq -r '.message // .' "$race/$racer" 2> /dev/null || cat "$race/$racer")" "$R"
  fi
done
mia_at=$(jq -r '.index // empty' "$race/Mia" 2> /dev/null || true)
olga_at=$(jq -r '.index // empty' "$race/Olga" 2> /dev/null || true)
if [[ -n $mia_at && $mia_at == "$olga_at" ]]; then
  echo "  ${YEL}Same height, different blocks: the chain has forked.${R} Heaviest chain wins, ties go to the lower hash."
elif [[ -n $mia_at && -n $olga_at ]]; then
  echo "  ${D}No fork this time: one block reached the other ${MINER}'s node first, so the second was built on top of it. Both count. Run it again for a split.${R}"
else
  echo "  ${D}No fork this time: one block arrived before the other ${MINER} had theirs, so that one was wasted. Run it again for a split.${R}"
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

act "Act 8 · A light client" "A phone can't keep every block. It keeps only their headers, and asks for a proof that a payment is in one. The block's hash covers a Merkle root, and the proof is the few hashes that lead from the payment up to it."
sha256() { printf '%s' "$1" | shasum -a 256 | cut -d' ' -f1; }
read -r from to amount sig <<< "${SENT[0]}"
headers=$(curl -s -m 10 "${NODES[1]}/headers")
printf '  📥 the phone follows %s headers: %s%s KB%s, where the whole chain is %s KB\n' "$(jq length <<< "$headers")" \
  "$B" "$(( $(printf '%s' "$headers" | wc -c) / 1024 ))" "$R" "$(( $(curl -s -m 10 "${NODES[1]}/chain" | wc -c) / 1024 ))"
printf '  🧾 it asks %s to prove %s paid %s %s coins\n' "$(node_label 1)" "${NAMES[$from]}" "${NAMES[$to]}" "$amount"
call GET "${NODES[1]}/transactions/$sig/proof"
if [[ $CODE != 200 ]]; then
  echo "  ${YEL}✗ $CODE $(jq -r '.message // .' <<< "$BODY")${R}"
else
  proof=$BODY
  hash=$(sha256 "leaf|$(jq -c .transaction <<< "$proof")")
  printf '     leaf      %s…  %s\n' "${hash:0:16}" "${D}sha256 of the transaction${R}"
  while read -r side sibling; do
    if [[ $side == left ]]; then hash=$(sha256 "node|$sibling|$hash"); else hash=$(sha256 "node|$hash|$sibling"); fi
    printf '     ↑ with    %s…  %s\n' "${sibling:0:16}" "${D}sibling on the $side, from the proof${R}"
    printf '     node      %s…\n' "${hash:0:16}"
  done < <(jq -r '.proof[] | "\(.side) \(.hash)"' <<< "$proof")
  root=$(jq -r .header.merkle_root <<< "$proof")
  index=$(jq -r .header.index <<< "$proof")
  known=$(jq -r --argjson i "$index" '.[] | select(.index == $i) | .hash' <<< "$headers")
  txs=$(curl -s -m 10 "${NODES[1]}/chain?from=$index" | jq '.[0].transactions | length')
  if [[ $hash == "$root" && $known == $(jq -r .header.hash <<< "$proof") ]]; then
    printf '  %s✓ it reaches the Merkle root of block #%s, whose header the phone already has.%s\n' "$GRN" "$index" "$R"
    printf '     %s hashes checked instead of downloading the block'"'"'s %s transactions. %s confirmations.\n' \
      "$(jq '.proof | length' <<< "$proof")" "$txs" "$(jq -r .confirmations <<< "$proof")"
  else
    echo "  ${RED}✗ the proof does not check out${R}"
  fi
  printf '  🔎 the same check in the explorer: %s/explorer.html#/tx/%s\n' "${NODES[1]}" "$sig"
fi

act "Epilogue" "Where everyone stands, according to every node. Paste an address into the explorer's search, or open its link."
explorer="${NODES[0]}/explorer.html"
for i in "${!NAMES[@]}"; do
  seen=()
  for n in "${!NODES[@]}"; do seen+=("$(balance "$i" "$n")"); done
  if [[ $(printf '%s\n' "${seen[@]}" | sort -u | wc -l) -eq 1 ]]; then
    agree="${D}every node agrees${R}"
  else
    agree="${RED}nodes disagree: ${seen[*]}${R}"
  fi
  printf '  %-6s %s%5s coins%s  %s, %s\n' "${NAMES[$i]}" "$B" "${seen[0]}" "$R" "${D}sent ${NONCE[$i]} transfer(s)${R}" "$agree"
  printf '         %s%s%s\n         %s#/address/%s\n' "$YEL" "${ADDR[$i]}" "$R" "$explorer" "${ADDR[$i]}"
done
echo
printf '  🔎 explorer: %s   latest block: %s#/block/%s\n' "$explorer" "$explorer" "$(status 0 | jq -r .height)"
echo
network
for i in "${!NODES[@]}"; do
  validity=$(curl -s -m 30 "${NODES[$i]}/chain/valid")
  if [[ $(jq -r .valid <<< "$validity") == true ]]; then
    echo "  ✅ $(node_label "$i") replays its whole chain from genesis: valid"
  else
    echo "  ❌ $(node_label "$i") replays its whole chain from genesis: ${RED}$(jq -r '.problem // "invalid"' <<< "$validity")${R}"
  fi
done
