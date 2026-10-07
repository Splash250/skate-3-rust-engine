-- Wallet records use verified platform account IDs. Connection actor IDs are
-- resolved from current admitted-player snapshots and never enter the ledger.
local ready = false
local sequence = 0
local pending, inflight, balances, receipts, unknowns = {}, {}, {}, {}, {}
local MAX_PENDING = 64

local function text(value) return {type='text', value=value} end
local function integer(value) return {type='integer', value=value} end
local function sql(query, params) return {sql=query, params=params} end
local function op_key(account, operation_id) return account .. '\0' .. operation_id end
local function is_integer(value) return type(value) == 'number' and value % 1 == 0 end
local function live_identity(actor)
    if type(actor) ~= 'string' or not actor:match('^[1-9][0-9]*$') then return nil end
    local account = resource.call('platform-profiles', 'identity', actor)
    if type(account) ~= 'string' or #account < 1 or #account > 160 then return nil end
    return account
end
local function result_for(receipt)
    return {
        ok=true, status=receipt.status, operation_id=receipt.operation_id,
        kind=receipt.kind, amount=receipt.amount, reason=receipt.reason,
        balance=receipt.balance, error=receipt.error
    }
end
local function publish(actor, account, balance, receipt)
    if live_identity(actor) ~= account then return end
    if balance ~= nil then resource.state.set('balance', {balance=balance}, {kind='player', id=actor}) end
    if receipt then resource.state.set('operation', receipt, {kind='player', id=actor}) end
end
local function enqueue(account, actor, request_type, operation_id, body, operation)
    if next(pending) and (function() local n=0; for _ in pairs(pending) do n=n+1 end; return n end)() >= MAX_PENDING then
        return false
    end
    local key = request_type .. '\0' .. account .. '\0' .. (operation_id or '')
    if inflight[key] then return true end
    sequence = sequence + 1
    local service_key = 'wallet_' .. tostring(sequence)
    pending[service_key] = {
        key=key, account=account, actor=actor, type=request_type,
        operation_id=operation_id, body=body
    }
    inflight[key] = service_key
    resource.services.submit(service_key, operation, 5000)
    return true
end
local function valid_operation_id(value)
    return type(value) == 'string' and #value >= 1 and #value <= 64 and value:match('^[%w._:-]+$') ~= nil
end
local function valid_request(payload)
    if type(payload) ~= 'table' or not valid_operation_id(payload.operation_id) then return nil end
    if payload.kind ~= 'charge' and payload.kind ~= 'credit' then return nil end
    if not is_integer(payload.amount) or payload.amount < 1 or payload.amount > resource.settings.get('max_operation_amount') then return nil end
    if type(payload.reason) ~= 'string' or #payload.reason < 1 or #payload.reason > 120 then return nil end
    return {operation_id=payload.operation_id, kind=payload.kind, amount=payload.amount, reason=payload.reason}
end
local function cached_response(account, body)
    local receipt = receipts[op_key(account, body.operation_id)]
    if not receipt then return nil end
    if receipt.kind ~= body.kind or receipt.amount ~= body.amount or receipt.reason ~= body.reason then
        return {ok=false, error='operation_conflict'}
    end
    return result_for(receipt)
end
local function balance(payload)
    if type(payload) ~= 'table' then return {ok=false, error='invalid_request'} end
    local actor = payload.actor
    local account = live_identity(actor)
    if not account then return {ok=false, error='unverified_actor'} end
    if not ready then return {ok=false, error='not_ready'} end
    if balances[account] ~= nil then return {ok=true, status='ready', balance=balances[account]} end
    local statements = {
        sql('INSERT INTO rp_wallets(account,balance) VALUES(?1,?2) ON CONFLICT(account) DO NOTHING', {text(account), integer(resource.settings.get('starter_balance'))}),
        sql('SELECT balance FROM rp_wallets WHERE account=?1', {text(account)})
    }
    if not enqueue(account, actor, 'balance', nil, nil, {kind='transaction', statements=statements}) then
        return {ok=false, error='busy'}
    end
    return {ok=true, status='pending'}
end
local function submit(payload)
    if type(payload) ~= 'table' then return {ok=false, error='invalid_request'} end
    local actor = payload.actor
    local account = live_identity(actor)
    if not account then return {ok=false, error='unverified_actor'} end
    if not ready then return {ok=false, error='not_ready'} end
    local body = valid_request(payload)
    if not body then return {ok=false, error='invalid_operation'} end
    local maximum = resource.settings.get('max_balance')
    if body.amount > maximum then return {ok=false, error='invalid_operation'} end
    local cached = cached_response(account, body)
    if cached then return cached end
    local key = op_key(account, body.operation_id)
    unknowns[key] = nil
    if inflight['submit\0' .. key] or inflight['lookup\0' .. key] then return {ok=true, status='pending', operation_id=body.operation_id} end
    local statements = {
        sql('INSERT INTO rp_wallets(account,balance) VALUES(?1,?2) ON CONFLICT(account) DO NOTHING', {text(account), integer(resource.settings.get('starter_balance'))}),
        sql("INSERT INTO rp_wallet_operations(account,operation_id,kind,amount,reason,status,error,balance) VALUES(?1,?2,?3,?4,?5,'pending',NULL,NULL) ON CONFLICT(account,operation_id) DO NOTHING", {text(account), text(body.operation_id), text(body.kind), integer(body.amount), text(body.reason)}),
        sql("UPDATE rp_wallets SET balance=CASE WHEN ?3='charge' THEN balance-?4 ELSE balance+?4 END WHERE account=?1 AND changes()=1 AND ((?3='charge' AND balance>=?4) OR (?3='credit' AND balance<=?5-?4))", {text(account), text(body.operation_id), text(body.kind), integer(body.amount), integer(maximum)}),
        sql("UPDATE rp_wallet_operations SET status=CASE WHEN changes()=1 THEN 'applied' ELSE 'rejected' END,error=CASE WHEN changes()=1 THEN NULL WHEN kind='charge' THEN 'insufficient_funds' ELSE 'balance_limit' END,balance=(SELECT balance FROM rp_wallets WHERE account=?1) WHERE account=?1 AND operation_id=?2 AND status='pending'", {text(account), text(body.operation_id) }),
        sql('SELECT operation_id,kind,amount,reason,status,error,balance FROM rp_wallet_operations WHERE account=?1 AND operation_id=?2', {text(account), text(body.operation_id)})
    }
    if not enqueue(account, actor, 'submit', body.operation_id, body, {kind='transaction', statements=statements}) then
        return {ok=false, error='busy'}
    end
    return {ok=true, status='pending', operation_id=body.operation_id}
end
local function operation(payload)
    if type(payload) ~= 'table' or not valid_operation_id(payload.operation_id) then return {ok=false, error='invalid_request'} end
    local actor = payload.actor
    local account = live_identity(actor)
    if not account then return {ok=false, error='unverified_actor'} end
    if not ready then return {ok=false, error='not_ready'} end
    local key = op_key(account, payload.operation_id)
    local receipt = receipts[key]
    if receipt then return result_for(receipt) end
    if unknowns[key] then return {ok=true, status='unknown', operation_id=payload.operation_id} end
    if inflight['lookup\0' .. key] or inflight['submit\0' .. key] then
        return {ok=true, status='pending', operation_id=payload.operation_id}
    end
    local query = {
        kind='query',
        statement=sql('SELECT operation_id,kind,amount,reason,status,error,balance FROM rp_wallet_operations WHERE account=?1 AND operation_id=?2', {text(account), text(payload.operation_id)})
    }
    if not enqueue(account, actor, 'lookup', payload.operation_id, nil, query) then return {ok=false, error='busy'} end
    return {ok=true, status='pending', operation_id=payload.operation_id}
end

resource.export('balance', balance)
resource.export('submit', submit)
resource.export('operation', operation)

local function decode_receipt(row)
    local function value(cell) return type(cell) == 'table' and cell.value or cell end
    return {
        operation_id=value(row[1]), kind=value(row[2]), amount=value(row[3]),
        reason=value(row[4]), status=value(row[5]), error=value(row[6]), balance=value(row[7])
    }
end
local function handle_receipt(request, rows)
    if #rows == 0 then
        unknowns[op_key(request.account, request.operation_id)] = true
        if request.actor and live_identity(request.actor) == request.account then
            resource.state.set('operation', {operation_id=request.operation_id,status='unknown'}, {kind='player',id=request.actor})
        end
        return
    end
    local receipt = decode_receipt(rows[1])
    local key = op_key(request.account, receipt.operation_id)
    unknowns[key] = nil
    receipts[key] = receipt
    balances[request.account] = receipt.balance
    publish(request.actor, request.account, receipt.balance, receipt)
end
resource.on('service_result', function(completion, sender)
    if sender ~= '0' then return end
    local result = completion.result
    if completion.key == 'schema' then
        ready = result.ok == true
        resource.state.set('ready', ready)
        if not ready then sdk.log('RP economy schema unavailable') end
        return
    end
    local request = pending[completion.key]
    if not request then return end
    pending[completion.key] = nil
    inflight[request.key] = nil
    if not result.ok then
        if request.actor and live_identity(request.actor) == request.account then
            local value = {status='error', message='Wallet operation could not be confirmed; retry with the same operation ID.'}
            if request.operation_id then value.operation_id=request.operation_id end
            resource.state.set(request.type == 'balance' and 'balance' or 'operation', value, {kind='player',id=request.actor})
        end
        return
    end
    if request.type == 'balance' then
        local rows = result.value.results[2].rows
        if #rows == 0 then return end
        balances[request.account] = rows[1][1].value
        publish(request.actor, request.account, balances[request.account], nil)
    elseif request.type == 'submit' then
        handle_receipt(request, result.value.results[#result.value.results].rows)
    elseif request.type == 'lookup' then
        handle_receipt(request, result.value.results[1].rows)
    end
end)

return {on_load=function()
    local starter = resource.settings.get('starter_balance')
    local maximum = resource.settings.get('max_balance')
    if not is_integer(starter) or not is_integer(maximum) or starter > maximum then
        resource.state.set('ready', false)
        sdk.log('RP economy settings are inconsistent')
        return
    end
    resource.services.submit('schema', {kind='migrate', migrations={{version=1, statements={
        sql('CREATE TABLE rp_wallets(account TEXT PRIMARY KEY,balance INTEGER NOT NULL CHECK(balance>=0 AND balance<=1000000000))'),
        sql("CREATE TABLE rp_wallet_operations(account TEXT NOT NULL REFERENCES rp_wallets(account),operation_id TEXT NOT NULL,kind TEXT NOT NULL CHECK(kind IN ('charge','credit')),amount INTEGER NOT NULL CHECK(amount>0),reason TEXT NOT NULL CHECK(length(reason) BETWEEN 1 AND 120),status TEXT NOT NULL CHECK(status IN ('pending','applied','rejected')),error TEXT,balance INTEGER CHECK(balance IS NULL OR balance>=0),PRIMARY KEY(account,operation_id))")
    }}}}, 5000)
end}
