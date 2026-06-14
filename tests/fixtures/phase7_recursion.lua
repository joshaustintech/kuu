local function sum(n, acc)
  if n == 0 then
    return acc
  end

  return sum(n - 1, acc + n)
end

print(sum(5, 0))
