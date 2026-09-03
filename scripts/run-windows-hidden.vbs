Option Explicit

Dim shell, fileSystem, scriptDirectory, projectRoot
Dim runnerPath, configPath, command

Set shell = CreateObject("WScript.Shell")
Set fileSystem = CreateObject("Scripting.FileSystemObject")

scriptDirectory = fileSystem.GetParentFolderName(WScript.ScriptFullName)
projectRoot = fileSystem.GetParentFolderName(scriptDirectory)
runnerPath = fileSystem.BuildPath(scriptDirectory, "run-windows-with-log.ps1")

If WScript.Arguments.Count > 0 Then
    configPath = WScript.Arguments(0)
Else
    configPath = fileSystem.BuildPath(projectRoot, "edgemouse.toml")
End If

command = "powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File " _
    & QuoteArgument(runnerPath) & " -ConfigPath " & QuoteArgument(configPath)

shell.CurrentDirectory = projectRoot
shell.Run command, 0, False

Function QuoteArgument(value)
    QuoteArgument = Chr(34) & Replace(value, Chr(34), Chr(34) & Chr(34)) & Chr(34)
End Function
