local ready, schema_ready = false, false
local sequence, service_sequence, clock, player_cursor = resource.storage.get('lease_sequence') or 0, 0, 0, 1
local leases, pending, invitations, used_instances = {}, {}, {}, {}
local trips, published, requests, invite_busy = {}, {}, {}, {}
local MAX_PENDING, INSTANCE_FIRST, INSTANCE_LAST = 64, 2000, 2063
local PLAYERS_PER_TICK = 12
local UNIT = {id='studio', label='Borough Studio', position={180,1,0}}

local function text(value) return {type='text',value=value} end
local function integer(value) return {type='integer',value=value} end
local function sql(query, params) if params then return {sql=query,params=params} end;return {sql=query} end
local function pair(owner, guest) return owner .. '\0' .. guest end
local function valid_actor(actor) return type(actor)=='string' and #actor<=20 and actor:match('^[1-9][0-9]*$')~=nil end
local function identity(actor)
    if not valid_actor(actor) then return nil end
    local account=resource.call('platform-profiles','identity',actor)
    return type(account)=='string' and #account>0 and #account<=160 and account or nil
end
local function actor_for(account)
    for _,player in ipairs(resource.players()) do
        if player.account_id==account and identity(player.id)==account then return player end
    end
end
local function cell(value) return type(value)=='table' and value.value or value end
local function rows_at(result,index) return result.value.results[index].rows end
local function rows_for_account(result,account)
    local values={}
    for _,row in ipairs(result) do if cell(row[1])==account then values[#values+1]=row end end
    return values
end
local function count_pending()
    local count=0;for _ in pairs(requests) do count=count+1 end;return count
end
local function queue(kind, account, actor, data, operation)
    if count_pending()>=MAX_PENDING then return false end
    service_sequence=service_sequence+1
    local key='property_'..tostring(service_sequence)
    requests[key]={kind=kind,account=account,actor=actor,data=data}
    resource.services.submit(key,operation,5000)
    return true
end
local function invitation_list(account)
    local result={}
    for _,inv in pairs(invitations) do
        local lease=leases[inv.owner]
        if inv.guest==account and lease and lease.operation_id==inv.operation_id then
            result[#result+1]={owner=inv.owner,unit=UNIT.label}
        end
    end
    table.sort(result,function(a,b)return a.owner<b.owner end)
    return result
end
local function online_players(actor)
    local result={}
    for _,player in ipairs(resource.players()) do
        if player.id~=actor and identity(player.id) then
            local profile=resource.call('platform-profiles','profile',player.id)
            local name=type(profile)=='table' and type(profile.name)=='string' and profile.name or 'Player'
            result[#result+1]={id=player.id,label=name..' (…'..string.sub(player.id,-4)..')'}
        end
    end
    table.sort(result,function(a,b) if a.label~=b.label then return a.label<b.label end return a.id<b.id end)
    return result
end
local function snapshot(account, actor, message, error)
    local lease=leases[account]
    local wallet=resource.call('rp-economy','balance',{actor=actor})
    local result={ready=ready,unit={id=UNIT.id,label=UNIT.label,price=resource.settings.get('studio_price')},
        lease=lease and {unit=lease.unit,operation_id=lease.operation_id,instance=lease.instance,label=UNIT.label} or nil,
        invitations=invitation_list(account),online_players=online_players(actor),in_property=trips[actor]~=nil,
        balance=wallet and wallet.balance, balance_status=wallet and wallet.status or 'pending',
        message=message,error=error}
    return result
end
local function publish(actor,account,message,error)
    if identity(actor)~=account then return end
    local value=snapshot(account,actor,message,error)
    resource.state.set('properties',value,{kind='player',id=actor})
    resource.send('response',value,actor)
    return value
end
local function reject(actor,account,code,message)
    publish(actor,account,message,code)
end
local function reserve_instance()
    for instance=INSTANCE_FIRST,INSTANCE_LAST do if not used_instances[instance] then return instance end end
end
local function request_state(actor,account,action)
    if action=='rent' then
        if leases[account] then return reject(actor,account,'already_leased','You already have a studio lease.') end
        if pending[account] then return publish(actor,account,'Your lease request is being processed.') end
        if type(UNIT.id)~='string' then return reject(actor,account,'unit_unavailable','That property is unavailable.') end
        local instance=reserve_instance()
        if not instance then return reject(actor,account,'capacity','All private apartments are occupied.') end
        sequence=sequence+1;resource.storage.set('lease_sequence',sequence)
        local record={account=account,actor=actor,unit=UNIT.id,operation_id='studio_'..tostring(sequence),
            amount=resource.settings.get('studio_price'),instance=instance,persisted=false,reserving=true,next_poll=0}
        pending[account]=record;used_instances[instance]=account
        local operation={kind='transaction',statements={
            sql('INSERT INTO rp_property_pending(account,unit,operation_id,amount,instance) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(account) DO NOTHING',
                {text(account),text(record.unit),text(record.operation_id),integer(record.amount),integer(instance)}),
            sql('SELECT account,unit,operation_id,amount,instance FROM rp_property_pending WHERE account=?1',{text(account)})
        }}
        if not queue('reserve',account,actor,record,operation) then
            pending[account]=nil;used_instances[instance]=nil
            return reject(actor,account,'busy','The property service is busy. Try again.')
        end
        return publish(actor,account,'Checking the lease and wallet…')
    elseif action=='list' or action=='snapshot' then
        return publish(actor,account)
    end
end
local function load_database()
    queue('load',nil,nil,nil,{kind='transaction',statements={
        sql('SELECT account,unit,operation_id,instance FROM rp_property_leases ORDER BY account'),
        sql('SELECT account,unit,operation_id,amount,instance FROM rp_property_pending ORDER BY account'),
        sql('SELECT owner_account,guest_account,operation_id FROM rp_property_invites ORDER BY owner_account,guest_account')
    }})
end
local function activate(record)
    if record.activating or not record.persisted then return end
    record.activating=true
    local operation={kind='transaction',statements={
        sql('INSERT INTO rp_property_leases(account,unit,operation_id,instance) VALUES(?1,?2,?3,?4) ON CONFLICT(account) DO NOTHING',
            {text(record.account),text(record.unit),text(record.operation_id),integer(record.instance)}),
        sql('DELETE FROM rp_property_pending WHERE account=?1 AND operation_id=?2',{text(record.account),text(record.operation_id)}),
        sql('SELECT account,unit,operation_id,instance FROM rp_property_leases WHERE account=?1',{text(record.account)})
    }}
    if not queue('commit',record.account,record.actor,record,operation) then record.activating=false end
end
local function release(record)
    if record.releasing then return end
    record.releasing=true
    local operation={kind='transaction',statements={
        sql('DELETE FROM rp_property_pending WHERE account=?1 AND operation_id=?2',{text(record.account),text(record.operation_id)})
    }}
    if not queue('release',record.account,record.actor,record,operation) then record.releasing=false end
end
local function reconcile(record,actor)
    if not record.persisted or record.activating or record.releasing or clock<record.next_poll then return end
    record.actor=actor;record.next_poll=clock+0.5
    local status=resource.call('rp-economy','operation',{actor=actor,operation_id=record.operation_id})
    if type(status)~='table' then return end
    if status.status=='applied' then
        activate(record)
    elseif status.status=='rejected' then
        release(record)
    elseif status.status=='unknown' then
        local submitted=resource.call('rp-economy','submit',{actor=actor,operation_id=record.operation_id,
            kind='charge',amount=record.amount,reason='Borough studio lease'})
        if type(submitted)=='table' and submitted.ok==false and submitted.error~='not_ready' then
            publish(actor,record.account,'Wallet request will retry.',submitted.error)
        end
    end
end
local function enter(actor,account,owner,lease)
    if trips[actor] then return reject(actor,account,'already_inside','You are already inside a property.') end
    if type(lease)~='table' or type(lease.instance)~='number' or lease.instance<INSTANCE_FIRST or lease.instance>INSTANCE_LAST then
        return reject(actor,account,'not_authorized','You do not have access to that property.')
    end
    resource.teleport(actor,{position=UNIT.position,heading=0,velocity={0,0,0},instance=lease.instance,restore_on_stop=true})
    trips[actor]={account=account,owner=owner,instance=lease.instance}
    publish(actor,account,'Welcome to the Borough Studio.')
end
local function handle_request(value,sender)
    if sender=='0' or type(value)~='table' then return end
    local account=identity(sender)
    if not account then return end
    local action=value.action
    if not ready then return reject(sender,account,'not_ready','Properties are starting. Try again shortly.') end
    if action=='list' or action=='snapshot' then return publish(sender,account) end
    if action=='rent' then
        if value.unit~='studio' then return reject(sender,account,'unit_unavailable','That property is unavailable.') end
        return request_state(sender,account,'rent')
    elseif action=='enter' then
        local owner=account
        if type(value.owner)=='string' then owner=value.owner end
        local lease=leases[owner]
        if owner==account and lease then return enter(sender,account,owner,lease) end
        local inv=invitations[pair(owner,account)]
        if lease and inv and inv.operation_id==lease.operation_id then return enter(sender,account,owner,lease) end
        return reject(sender,account,'not_authorized','You do not have access to that property.')
    elseif action=='exit' then
        if not trips[sender] then return reject(sender,account,'not_inside','You are not inside a property.') end
        trips[sender]=nil
        resource.teleport(sender,{restore_previous=true})
        return publish(sender,account,'Returned to Boardwalk Borough.')
    elseif action=='invite' then
        local lease=leases[account]
        local target=type(value.target)=='string' and value.target or nil
        local target_account=target and identity(target)
        if not lease or not target_account or target_account==account then
            return reject(sender,account,'invalid_invite','Choose an online player to invite.')
        end
        local existing=invitations[pair(account,target_account)]
        if existing and existing.operation_id==lease.operation_id then return publish(sender,account,'That player already has access.') end
        local count=0;for _,inv in pairs(invitations) do if inv.owner==account and inv.operation_id==lease.operation_id then count=count+1 end end
        if count>=resource.settings.get('max_guests') then return reject(sender,account,'guest_limit','This apartment has reached its guest limit.') end
        local key=pair(account,target_account)
        if invite_busy[key] then return publish(sender,account,'Invitation is being saved…') end
        invite_busy[key]=true
        local data={owner=account,guest=target_account,operation_id=lease.operation_id,target=target}
        local operation={kind='transaction',statements={sql('INSERT INTO rp_property_invites(owner_account,guest_account,operation_id) VALUES(?1,?2,?3) ON CONFLICT(owner_account,guest_account) DO UPDATE SET operation_id=excluded.operation_id',
            {text(account),text(target_account),text(lease.operation_id)})}}
        if not queue('invite',account,sender,data,operation) then invite_busy[key]=nil;return reject(sender,account,'busy','The property service is busy. Try again.') end
        return publish(sender,account,'Saving invitation…')
    elseif action=='accept_invite' then
        if type(value.owner)~='string' or #value.owner>160 then return reject(sender,account,'invalid_invite','Choose a valid invitation.') end
        local owner=value.owner;local lease=leases[owner];local inv=invitations[pair(owner,account)]
        if not lease or not inv or inv.operation_id~=lease.operation_id then return reject(sender,account,'not_authorized','That invitation is no longer available.') end
        return enter(sender,account,owner,lease)
    end
end

resource.export('snapshot',function(actor)
    local account=identity(actor)
    if not account then return {ok=false,error='unverified_actor'} end
    return snapshot(account,actor)
end)
resource.on_net('request',handle_request)
resource.on('service_result',function(completion,sender)
    if sender~='0' then return end
    local result=completion.result
    if completion.key=='schema' then
        schema_ready=result.ok==true
        if schema_ready then load_database() else sdk.log('Property database schema unavailable') end
        return
    end
    local request=requests[completion.key]
    if not request then return end
    requests[completion.key]=nil
    if request.kind=='load' then
        if not result.ok then sdk.log('Property database load failed');return end
        leases={};pending={};invitations={};used_instances={}
        for _,row in ipairs(rows_at(result,1)) do
            local account=cell(row[1]);local lease={account=account,unit=cell(row[2]),operation_id=cell(row[3]),instance=cell(row[4])}
            leases[account]=lease;used_instances[lease.instance]=account
        end
        for _,row in ipairs(rows_at(result,2)) do
            local account=cell(row[1]);local record={account=account,unit=cell(row[2]),operation_id=cell(row[3]),amount=cell(row[4]),instance=cell(row[5]),persisted=true,next_poll=0}
            pending[account]=record;used_instances[record.instance]=account
        end
        for _,row in ipairs(rows_at(result,3)) do
            local owner=cell(row[1]);local guest=cell(row[2]);local operation_id=cell(row[3])
            invitations[pair(owner,guest)]={owner=owner,guest=guest,operation_id=operation_id}
        end
        for key,inv in pairs(invitations) do local lease=leases[inv.owner];if not lease or lease.operation_id~=inv.operation_id then invitations[key]=nil end end
        ready=true;resource.state.set('ready',true)
    elseif request.kind=='reserve' then
        local record=request.data
        record.reserving=false
        if not result.ok then record.next_poll=clock+1;return end
        local rows=rows_at(result,2)
        if #rows~=1 or cell(rows[1][3])~=record.operation_id then
            pending[record.account]=nil;used_instances[record.instance]=nil
            if identity(record.actor)==record.account then publish(record.actor,record.account,'A lease already exists or could not be reserved.','lease_conflict') end
            return
        end
        record.persisted=true
        if identity(record.actor)==record.account then publish(record.actor,record.account,'Lease reserved. Confirming payment…') end
    elseif request.kind=='commit' then
        local record=request.data
        if not result.ok then record.activating=false;record.next_poll=clock+1;return end
        local rows=rows_at(result,3)
        if #rows~=1 or cell(rows[1][3])~=record.operation_id then record.activating=false;return end
        local lease={account=record.account,unit=cell(rows[1][2]),operation_id=cell(rows[1][3]),instance=cell(rows[1][4])}
        leases[record.account]=lease;pending[record.account]=nil;used_instances[lease.instance]=record.account
        if identity(record.actor)==record.account then publish(record.actor,record.account,'Lease purchased. Welcome home.') end
    elseif request.kind=='release' then
        local record=request.data
        if result.ok then pending[record.account]=nil;used_instances[record.instance]=nil
            if identity(record.actor)==record.account then publish(record.actor,record.account,'Payment was declined. No lease was created.','payment_rejected') end
        else record.releasing=false;record.next_poll=clock+1 end
    elseif request.kind=='invite' then
        local data=request.data;invite_busy[pair(data.owner,data.guest)]=nil
        if result.ok then
            invitations[pair(data.owner,data.guest)]={owner=data.owner,guest=data.guest,operation_id=data.operation_id}
            if identity(request.actor)==request.account then publish(request.actor,request.account,'Invitation sent.') end
            local guest=actor_for(data.guest);if guest then publish(guest.id,data.guest,'You received an apartment invitation.') end
        elseif identity(request.actor)==request.account then publish(request.actor,request.account,'Invitation could not be saved. Try again.','database_error') end
    end
end)

return {
    on_load=function()
        resource.services.submit('schema',{kind='migrate',migrations={{version=1,statements={
            sql('CREATE TABLE rp_property_leases(account TEXT PRIMARY KEY,unit TEXT NOT NULL,operation_id TEXT NOT NULL UNIQUE,instance INTEGER NOT NULL UNIQUE CHECK(instance BETWEEN 2000 AND 2063))'),
            sql('CREATE TABLE rp_property_pending(account TEXT PRIMARY KEY,unit TEXT NOT NULL,operation_id TEXT NOT NULL UNIQUE,amount INTEGER NOT NULL CHECK(amount>0),instance INTEGER NOT NULL UNIQUE CHECK(instance BETWEEN 2000 AND 2063))'),
            sql('CREATE TABLE rp_property_invites(owner_account TEXT NOT NULL,guest_account TEXT NOT NULL,operation_id TEXT NOT NULL,PRIMARY KEY(owner_account,guest_account))')
        }}}},5000)
    end,
    on_update=function(frame)
        clock=clock+(type(frame)=='table' and type(frame.dt)=='number' and frame.dt or 0)
        if not ready then return end
        local players=resource.players()
        local live={}
        for _,player in ipairs(players) do live[player.id]=true end
        for actor in pairs(published) do if not live[actor] then published[actor]=nil;trips[actor]=nil end end
        local verified={}
        if #players>0 then
            for _=1,math.min(PLAYERS_PER_TICK,#players) do
                if player_cursor>#players then player_cursor=1 end
                local player=players[player_cursor];player_cursor=player_cursor+1
                local account=identity(player.id)
                if account then
                    verified[account]=player
                    if not published[player.id] then publish(player.id,account);published[player.id]=true end
                end
            end
        end
        for account,record in pairs(pending) do
            local player=verified[account]
            if player then
                if not record.persisted then
                    if not record.reserving and clock>=record.next_poll then
                        record.reserving=true
                        local operation={kind='transaction',statements={
                            sql('INSERT INTO rp_property_pending(account,unit,operation_id,amount,instance) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(account) DO NOTHING',{text(account),text(record.unit),text(record.operation_id),integer(record.amount),integer(record.instance)}),
                            sql('SELECT account,unit,operation_id,amount,instance FROM rp_property_pending WHERE account=?1',{text(account)})
                        }}
                        if not queue('reserve',account,player.id,record,operation) then record.reserving=false end
                    end
                else reconcile(record,player.id) end
            end
        end
    end
}
