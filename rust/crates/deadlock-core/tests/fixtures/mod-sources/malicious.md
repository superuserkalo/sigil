# Safe heading

<script>alert("raw")</script>
<style>body{display:none}</style>
<img src="https://evil.example/x.png" onerror="alert(1)">
<a href="javascript:alert(1)">click</a>

[bad](javascript:alert(1)) [plain](http://evil.example/x) [file](file:///etc/passwd)
![remote](https://evil.example/track.png)
<iframe src="https://evil.example"></iframe> <https://mods.example.org/ok>
<!-- hidden [x](https://evil.example) -->
&amp; &lt;b&gt;
