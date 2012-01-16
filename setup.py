"""
py2app/py2exe build script for Lightningbeam.

Will automatically ensure that all build prerequisites are available
via ez_setup

Usage (Mac OS X):
    python setup.py py2app

Usage (Windows):
    python setup.py py2exe
"""
import ez_setup
ez_setup.use_setuptools()

import sys
from setuptools import setup


mainscript = 'lightningbeam.py'

if sys.platform == 'darwin':
    extra_options = dict(
        setup_requires=['py2app'],
        app=[mainscript],
        # Cross-platform applications generally expect sys.argv to
        # be used for opening files.
        options=dict(py2app=dict(argv_emulation=False,
        plist=dict(
            #CFBundleDocumentTypes= ,
            CFBundleIdentifyer='org.lightningbeam.lightningbeam',
            #LSPrefersPPC=True,
        ),
        resources=["media","gpl.txt","swfc","base.js"],
        iconfile="Lightningbeam.icns"
        )),
    )
elif sys.platform == 'win32':
    import GUI.py2exe
    import py2exe
    import os
    import win32ui
    Mydata_files=["gpl.txt",]#"GUI",]
    for files in os.listdir('media'):
        f1 = 'media/' + files
        if os.path.isfile(f1): # skip directories
            f2 = 'media', [f1]
            Mydata_files.append(f2)
    for files in os.listdir('swfc'):
        f1 = 'swfc/' + files
        if os.path.isfile(f1): # skip directories
            f2 = 'swfc', [f1]
            Mydata_files.append(f2)
    extra_options = dict(
        setup_requires=['py2exe'],
        windows=[{"script":mainscript,"icon_resources":[(1,"media/icon.ico")]}],
		other_resources=[("media",["media"]),("gpl.txt",["gpl.txt"]),("swfc",["swfc"]),("GUI",["GUI"])],
        data_files=Mydata_files,
        options=dict(py2exe=dict(packages=["win32ui","win32clipboard","win32api","win32gui","win32process"],
		             skip_archive=True,)),
    )
else:
     extra_options = dict(
         # Normally unix-like platforms will use "setup.py install"
         # and install the main script as such
         scripts=[mainscript],
     )

setup(
    name="Lightningbeam",
    **extra_options
)