local function make()
  local x = 0
  return function()
    x = x + 1
    return x
  end
end

local inc = make()
print(inc(), inc(), inc())
