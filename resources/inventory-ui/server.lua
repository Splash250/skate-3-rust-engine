-- Account identity comes exclusively from the server's verified player snapshot.
local ready, serial = false, 0
local pending, busy = {}, {}
local catalog = {
    {id = "deck_blue", name = "Blue deck", description = "A spare for the next session.", price = 25},
    {id = "wheels_54", name = "54 mm wheels", description = "Keep a fresh set in your bag.", price = 40},
    {id = "grip_sheet", name = "Grip sheet", description = "New grip, ready when you need it.", price = 10}
}
local function text(value) return {type="text", value=value} end
local function integer(value) return {type="integer", value=value} end
local function sql(query, params) return {sql=query, params=params} end
local function reply(sender, value) resource.send("inventory", value, sender) end
local function identity(sender)
    for _, player in ipairs(resource.players()) do
        if player.id == sender and type(player.account_id) == "string" then return player.account_id end
    end
end
local function fail(sender, message) reply(sender, {ok=false, message=message}) end

resource.on_net("inventory_request", function(payload, sender)
    local account = identity(sender)
    if not account then fail(sender, "Sign in to this server to use your inventory."); return end
    if not ready then fail(sender, "Inventory is starting. Try Refresh shortly."); return end
    if busy[sender] then fail(sender, "Your previous request is still running."); return end
    if type(payload) ~= "table" or (payload.action ~= "inspect" and payload.action ~= "buy") then return end
    local count=0; for _ in pairs(pending) do count=count+1 end
    if count >= 64 then fail(sender, "Inventory is busy. Try Refresh shortly."); return end
    local statements = {sql("INSERT INTO accounts(id,coins) VALUES(?1,100) ON CONFLICT(id) DO NOTHING", {text(account)})}
    local bought
    if payload.action == "buy" then
        for _, item in ipairs(catalog) do if item.id == payload.item then bought=item end end
        if not bought then fail(sender, "That item is unavailable. Refresh your inventory."); return end
        statements[#statements+1]=sql("UPDATE accounts SET coins=coins-?2 WHERE id=?1", {text(account), integer(bought.price)})
        statements[#statements+1]=sql("INSERT INTO inventory(account,item,quantity) VALUES(?1,?2,1) ON CONFLICT(account,item) DO UPDATE SET quantity=quantity+1", {text(account), text(bought.id)})
    end
    statements[#statements+1]=sql("SELECT coins FROM accounts WHERE id=?1", {text(account)})
    statements[#statements+1]=sql("SELECT item,quantity FROM inventory WHERE account=?1 ORDER BY item", {text(account)})
    serial=serial+1
    local key="request_"..serial
    pending[key]={sender=sender, account=account, bought=bought and bought.name}
    busy[sender]=true
    resource.services.submit(key, {kind="transaction", statements=statements}, 5000)
end)
resource.on("service_result", function(payload)
    if payload.key == "schema" then
        ready=payload.result.ok
        sdk.log(ready and "Inventory ready" or ("Inventory database: "..payload.result.error.message))
        return
    end
    local request=pending[payload.key]
    if not request then return end
    pending[payload.key]=nil; busy[request.sender]=nil
    -- A late result must not leak into a new or revoked authenticated session.
    if identity(request.sender) ~= request.account then return end
    if not payload.result.ok then
        fail(request.sender, payload.result.error.message == "SQL constraint failed; transaction rolled back" and "Not enough coins, or your bag is full. Nothing was charged." or "Purchase could not be completed. Refresh and try again.")
        return
    end
    local results=payload.result.value.results
    local owned={}
    for _, row in ipairs(results[#results].rows) do owned[row[1].value]=row[2].value end
    reply(request.sender, {ok=true, coins=results[#results-1].rows[1][1].value, catalog=catalog, owned=owned,
        message=request.bought and (request.bought.." added to your bag.") or "Inventory is up to date."})
end)
return {on_load=function()
    resource.services.submit("schema", {kind="migrate", migrations={{version=1, statements={
        sql("CREATE TABLE accounts(id TEXT PRIMARY KEY, coins INTEGER NOT NULL CHECK(coins >= 0 AND coins <= 1000000000))"),
        sql("CREATE TABLE inventory(account TEXT NOT NULL REFERENCES accounts(id), item TEXT NOT NULL, quantity INTEGER NOT NULL CHECK(quantity > 0 AND quantity <= 9999), PRIMARY KEY(account,item))")
    }}}}, 5000)
end}
