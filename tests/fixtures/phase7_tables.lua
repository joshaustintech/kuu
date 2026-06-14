local t = {}
t[1] = 1
t.answer = 42
local u = t
print(t[1], t.answer, t == u, t[t] and "bad" or "ok")
