-- Local-console example. Account labels are administrator-selected data keys;
-- they are deliberately never accepted as authenticated identities from clients.
local ready, sequence = false, 0
local function text(value) return {type = "text", value = value} end
local function integer(value) return {type = "integer", value = value} end
local function statement(sql, params) return {sql = sql, params = params} end
local function submit(prefix, operation)
    sequence = sequence + 1
    resource.services.submit(prefix .. "_" .. sequence, operation, 5000)
end
local function account(args)
    if not ready then sdk.log("Progression schema is not ready"); return nil end
    local name = args[1]
    if type(name) ~= "string" or #name < 1 or #name > 64 or not name:match("^[%w_-]+$") then
        sdk.log("Supply an account label: 1-64 letters, digits, '_' or '-'")
        return nil
    end
    return name
end

resource.on("service_result", function(payload)
    local result = payload.result
    if not result.ok then
        sdk.log("Progression " .. payload.key .. ": " .. result.error.code .. " - " .. result.error.message)
        return
    end
    if payload.key == "schema" then
        ready = true
        sdk.log("Progression ready; console: command progress_award ACCOUNT 100")
    elseif payload.key:match("^inspect_") then
        local results = result.value.results
        local rows = results[1].rows
        if #rows == 0 then sdk.log("Unknown progression account"); return end
        sdk.log("Balance: " .. tostring(rows[1][1].value))
        for _, row in ipairs(results[2].rows) do
            sdk.log("Inventory " .. row[1].value .. ": " .. tostring(row[2].value))
        end
    else
        sdk.log("Progression " .. payload.key .. " committed")
    end
end)

resource.command("progress_award", "progression.admin", function(args)
    local name = account(args)
    local amount = tonumber(args[2])
    if not name then return end
    if not amount or amount % 1 ~= 0 or amount < 1 or amount > 10000 then
        sdk.log("Award must be an integer from 1 to 10000"); return
    end
    submit("award", {kind = "transaction", statements = {
        statement("INSERT INTO accounts(id,coins) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET coins=coins+excluded.coins", {text(name), integer(amount)})
    }})
end)
resource.command("progress_buy", "progression.admin", function(args)
    local name = account(args)
    if not name then return end
    submit("buy", {kind = "transaction", statements = {
        statement("UPDATE accounts SET coins=coins-25 WHERE id=?1", {text(name)}),
        statement("INSERT INTO inventory(account,item,quantity) VALUES(?1,'deck_blue',1) ON CONFLICT(account,item) DO UPDATE SET quantity=quantity+1", {text(name)})
    }})
end)
resource.command("progress_inspect", "progression.admin", function(args)
    local name = account(args)
    if not name then return end
    submit("inspect", {kind = "transaction", statements = {
        statement("SELECT coins FROM accounts WHERE id=?1", {text(name)}),
        statement("SELECT item,quantity FROM inventory WHERE account=?1 ORDER BY item", {text(name)})
    }})
end)

return {
    on_load = function()
        resource.services.submit("schema", {kind = "migrate", migrations = {
            {version = 1, statements = {
                statement("CREATE TABLE accounts(id TEXT PRIMARY KEY, coins INTEGER NOT NULL CHECK(coins >= 0 AND coins <= 1000000000))"),
                statement("CREATE TABLE inventory(account TEXT NOT NULL REFERENCES accounts(id), item TEXT NOT NULL, quantity INTEGER NOT NULL CHECK(quantity > 0 AND quantity <= 1000000), PRIMARY KEY(account,item))")
            }}
        }}, 5000)
    end
}
